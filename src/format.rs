use image::RgbaImage;

use crate::{Compression, Header, PixelFormat};

// Given a mask, we first take the bits we care about and shift them down to start at 0
// After that, we convert them to being in the range [0, 256)
fn uncompressed_convert_mask(pixel: u32, mask: u32) -> u8 {
  ((pixel & mask) >> mask.trailing_zeros() * 255 / (2u32.pow(mask.count_ones()) - 1)) as u8
}

// Handles decoding an uncompressed buffer into a series of mipmap images
pub fn decode_layers_uncompressed(header: &Header, mut buf: &[u8]) -> Vec<RgbaImage> {
  let layer_sizes = header.get_layer_sizes();
  let mut layers = Vec::with_capacity(layer_sizes.len());
  for (h, w) in layer_sizes {
    let layer_size = h * w * header.pixel_bytes;
    let (layer_data, new_buf) = buf.split_at(layer_size);
    buf = new_buf;

    // Chunk into groups of 3 or 4, then convert to normalized RGBA format
    let mut layer = Vec::with_capacity(layer_data.len() / header.pixel_bytes);
    for p in layer_data.chunks(header.pixel_bytes) {
      let pixel: u32 = p[0] as u32 + ((p[1] as u32) << 8) + ((p[2] as u32) << 16) + ((p[3] as u32) << 24);

      layer.push([
        uncompressed_convert_mask(pixel, header.channel_masks[0]),
        uncompressed_convert_mask(pixel, header.channel_masks[1]),
        uncompressed_convert_mask(pixel, header.channel_masks[2]),
        uncompressed_convert_mask(pixel, header.channel_masks[3])
      ]);
    };

    let layer = pixels_into_bytes(layer);
    let layer = RgbaImage::from_raw(w as u32, h as u32, layer)
      .expect("error converting bytes to image buffer");
    layers.push(layer);
  };

  layers
}

// Implements this lookup table for calculating pixel colors
//
// code | color0 > color1 | color0 <= color1
// -----------------------------------------
//   0  |       c0        |       c0
//   1  |       c1        |       c1
//   2  | (2*c0 + c1) / 3 |  (c0 + c1) / 2
//   3  | (c0 + 2*c1) / 3 |      black
//
// Returns an Option to differentiate between a black pixel and a transparent pixel
fn dxt1_lookup(key: (bool, u8), c0: u32, c1: u32, inflate_by: u32) -> Option<u32> {
  // Inflate colors from 5/6-bit to 8-bit
  let c0 = c0 * 255 / (2u32.pow(inflate_by) - 1);
  let c1 = c1 * 255 / (2u32.pow(inflate_by) - 1);

  match key {
    (true, 0) => Some(c0),
    (true, 1) => Some(c1),
    (true, 2) => Some((2 * c0 + c1) / 3),
    (true, 3) => Some((c0 + 2 * c1) / 3),
    (false, 0) => Some(c0),
    (false, 1) => Some(c1),
    (false, 2) => Some((c0 + c1) / 2),
    (false, 3) => None,
    _ => unreachable!()
  }
}

// Handles decoding a DXT1-compressed 64-bit buffer into 16 pixels. Handles 1-bit alpha variant with `alpha` parameter
fn decode_chunk_dxt1(bytes: &[u8], alpha: bool) -> Vec<[u8; 4]> {
  // Convert to `u32` to allow overflow for arithmetic below
  let color0 = (((bytes[1] as u16) << 8) + bytes[0] as u16) as u32;
  let color1 = (((bytes[3] as u16) << 8) + bytes[2] as u16) as u32;

  // Iterate through each pair of bits in each `code` byte to
  // determine the color for each pixel
  let mut layer = Vec::with_capacity((bytes.len() - 4) * 4);
  for &code in bytes[4..].iter().rev() {
    for i in 0..4 {
      let red0 = (color0 & 0xF800) >> 11;
      let red1 = (color1 & 0xF800) >> 11;
      let green0 = (color0 & 0x7E0) >> 5;
      let green1 = (color1 & 0x7E0) >> 5;
      let blue0 = color0 & 0x1F;
      let blue1 = color1 & 0x1F;

      // If alpha is disabled or if any channel is non-black, show the pixel, otherwise, hide it
      let key = (color0 > color1, (code >> (i * 2)) & 0x3);
      let r = dxt1_lookup(key, red0, red1, 5);
      let g = dxt1_lookup(key, green0, green1, 6);
      let b = dxt1_lookup(key, blue0, blue1, 5);
      let a = if !alpha || r.is_some() || g.is_some() || b.is_some() { 255 } else { 0 };

      layer.push([
        r.unwrap_or(0) as u8,
        g.unwrap_or(0) as u8,
        b.unwrap_or(0) as u8,
        a
      ]);
    };
  };

  layer
}

fn dxt3_lookup(key: u8, c0: u32, c1: u32, inflate_by: u32) -> u32 {
  // Inflate colors from 5/6-bit to 8-bit
  let c0 = c0 * 255 / (2u32.pow(inflate_by) - 1);
  let c1 = c1 * 255 / (2u32.pow(inflate_by) - 1);

  match key {
    0 => c0,
    1 => c1,
    2 => (2 * c0 + c1) / 3,
    3 => (c0 + 2 * c1) / 3,
    _ => unreachable!()
  }
}

// Handles decoding a DXT2/3-compressed 128-bit buffer into 16 pixels
fn decode_chunk_dxt3(bytes: &[u8]) -> Vec<[u8; 4]> {
  // Convert to `u32` to allow overflow for arithmetic below
  let color0 = (((bytes[9] as u16) << 8) + bytes[8] as u16) as u32;
  let color1 = (((bytes[11] as u16) << 8) + bytes[10] as u16) as u32;

  // Iterate through each pair of bits in each `code` byte to determine the color for each pixel
  let mut layer = Vec::with_capacity((bytes.len() - 12) * 4);
  for (i, &code) in bytes[12..].iter().rev().enumerate() {
    for j in 0..4 {
      let alpha_nibble = (bytes[2 * (3 - i) + j / 2] >> (4 * (j % 2))) & 0xF;

      let red0 = (color0 & 0xF800) >> 11;
      let red1 = (color1 & 0xF800) >> 11;
      let green0 = (color0 & 0x7E0) >> 5;
      let green1 = (color1 & 0x7E0) >> 5;
      let blue0 = color0 & 0x1F;
      let blue1 = color1 & 0x1F;

      let key = (code >> (j * 2)) & 0x3;
      layer.push([
        dxt3_lookup(key, red0, red1, 5) as u8,
        dxt3_lookup(key, green0, green1, 6) as u8,
        dxt3_lookup(key, blue0, blue1, 5) as u8,
        (alpha_nibble as u32 * 255 / 15) as u8
      ]);
    };
  };

  layer
}

// Implements this lookup table for calculating pixel colors
//
// code |      value      |
// ------------------------
//   0  |       c0        |
//   1  |       c1        |
//   2  | (2*c0 + c1) / 3 |
//   3  | (c0 + 2*c1) / 3 |
fn dxt5_lookup(key: u8, c0: u32, c1: u32, inflate_by: u32) -> u32 {
  // Inflate colors from 5/6-bit to 8-bit
  let c0 = c0 * 255 / (2u32.pow(inflate_by) - 1);
  let c1 = c1 * 255 / (2u32.pow(inflate_by) - 1);

  match key {
    0 => c0,
    1 => c1,
    2 => (2 * c0 + c1) / 3,
    3 => (c0 + 2 * c1) / 3,
    _ => unreachable!()
  }
}

// Interpolate between two given alpha values based on the 3-bit lookup value stored in `alpha_info`
fn dxt5_alpha_interp(alpha0: u32, alpha1: u32, key: u64) -> u32 {
  match (alpha0 > alpha1, key) {
    (true, 0) => alpha0,
    (true, 1) => alpha1,
    (true, 2) => (6 * alpha0 + 1 * alpha1) / 7,
    (true, 3) => (5 * alpha0 + 2 * alpha1) / 7,
    (true, 4) => (4 * alpha0 + 3 * alpha1) / 7,
    (true, 5) => (3 * alpha0 + 4 * alpha1) / 7,
    (true, 6) => (2 * alpha0 + 5 * alpha1) / 7,
    (true, 7) => (1 * alpha0 + 6 * alpha1) / 7,
    (false, 0) => alpha0,
    (false, 1) => alpha1,
    (false, 2) => (4 * alpha0 + 1 * alpha1) / 5,
    (false, 3) => (3 * alpha0 + 2 * alpha1) / 5,
    (false, 4) => (2 * alpha0 + 3 * alpha1) / 5,
    (false, 5) => (1 * alpha0 + 4 * alpha1) / 5,
    (false, 6) => 0,
    (false, 7) => 255,
    t => unreachable!("Unexpected value: {:?}", t)
  }
}

// Handles decoding a DXT4/5-compressed 128-bit buffer into 16 pixels
fn decode_chunk_dxt5(bytes: &[u8]) -> Vec<[u8; 4]> {
  let color0 = (((bytes[9] as u16) << 8) + bytes[8] as u16) as u32;
  let color1 = (((bytes[11] as u16) << 8) + bytes[10] as u16) as u32;

  let alpha0 = bytes[0] as u32;
  let alpha1 = bytes[1] as u32;

  // Convert 6 u8's into a single 48 bit number, to make it easier to grab 3-bit chunks out of them
  let alpha_info = bytes[2..8].iter().enumerate()
    .fold(0u64, |memo, (i, &x)| memo + ((x as u64) << 8 * i) as u64);
  let mut layer = Vec::with_capacity((bytes.len() - 12) * 4);
  for (i, &code) in bytes[12..].iter().rev().enumerate() {
    for j in 0..4 {
      let red0 = (color0 & 0xF800) >> 11;
      let red1 = (color1 & 0xF800) >> 11;
      let green0 = (color0 & 0x7E0) >> 5;
      let green1 = (color1 & 0x7E0) >> 5;
      let blue0 = color0 & 0x1F;
      let blue1 = color1 & 0x1F;

      let alpha_key = (alpha_info >> (3 * (4 * (3 - i) + j))) & 0x07;
      let alpha = dxt5_alpha_interp(alpha0, alpha1, alpha_key);

      let key = (code >> (j * 2)) & 0x3;
      layer.push([
        dxt5_lookup(key, red0, red1, 5) as u8,
        dxt5_lookup(key, green0, green1, 6) as u8,
        dxt5_lookup(key, blue0, blue1, 5) as u8,
        alpha as u8
      ]);
    };
  };

  layer
}

fn dxt_chunk_transform(chunk: &[u8], header: &Header) -> Vec<[u8; 4]> {
  match header.compression {
    Compression::DXT1 => decode_chunk_dxt1(chunk, header.pixel_format != PixelFormat::Unknown),
    Compression::DXT2 | Compression::DXT3 => decode_chunk_dxt3(chunk),
    Compression::DXT4 | Compression::DXT5 => decode_chunk_dxt5(chunk),
    _ => unreachable!(format!("This function cannot handle `{:?}` images", header.compression))
  }
}

fn dxt_transpose_texels(chunk: &[[u8; 4]], w: usize, width: usize) -> Vec<[u8; 4]> {
  let mut pixels = Vec::new();
  for i in (0..4).rev() {
    for j in 0..(w / 4) {
      // If this is the last block in a row and the image width is not evenly divisible by 4, we
      // only push enough pixels to fill the rest of the block width
      let block_width = if j + 1 == w / 4 { 4 - (w - width) } else { 4 };

      for k in 0..block_width {
        pixels.push(chunk[(i + j * 4) * 4 + k]);
      }
    }
  }

  pixels
}

// Handles decoding a DXT1-5 compressed buffer into a series of mipmap images
pub fn decode_layers_dxt(header: &Header, mut buf: &[u8]) -> Vec<RgbaImage> {
  let layer_sizes = header.get_layer_sizes();
  let mut layers = Vec::with_capacity(layer_sizes.len());
  for (height, width) in layer_sizes {
    // We calculate the actual height and width here. Although the given height/width
    // can go down to 1, the block sizes are minimum 4x4, which we enforce here. We
    // then also round up to the nearest even divisor of 4. For example, a 47x49 texture
    // is actually stored as a 48x52 texture.
    let h = (height.max(4) as f32 / 4.0).ceil() as usize * 4;
    let w = (width.max(4) as f32 / 4.0).ceil() as usize * 4;

    // DXT1 compression uses 64 bits per 16 pixels, while DXT2-5 use 128 bits.
    // Calculate how many total bytes to read out of the buffer for each layer
    // here, as well as how big each individual chunk size is.
    let (layer_size, chunk_size) = match header.compression {
      Compression::DXT1 => (h * w / 2, 8),
      _ => (h * w, 16)
    };

    let (layer_data, new_buf) = buf.split_at(layer_size);
    buf = new_buf;

    let layer = layer_data
      // Chunk into blocks of appropriate size
      .chunks(chunk_size)
      // Turn those blocks into 16 RGBA pixels, and flatten into a
      // vec of pixels for the entire image. Follow here for the dirty details:
      // https://www.khronos.org/opengl/wiki/S3_Texture_Compression
      .flat_map(|chunk| dxt_chunk_transform(chunk, header))
      .collect::<Vec<_>>()
      // Since the 16 byte pixel blocks are actually 4x4 texels, group image
      // into chunks of four rows each, and then transpose into a row of texels.
      .chunks(4 * w)
      .flat_map(|chunk| dxt_transpose_texels(chunk, w, width))
      .collect::<Vec<_>>();
    let mut layer = pixels_into_bytes(layer);
    layer.truncate(width * height * 4);
    layer.shrink_to_fit();
    // Layer's length is now equal to `width * height * 4`
    // `width` and `height` are now the buffer's real dimensions
    let layer = RgbaImage::from_raw(width as u32, height as u32, layer)
      .expect("error converting bytes to image buffer");
    layers.push(layer);
  };

  layers
}

pub fn decode_layers(header: &Header, buf: &[u8]) -> Result<Vec<RgbaImage>, Compression> {
  match header.compression {
    Compression::None => {
      Ok(decode_layers_uncompressed(&header, buf))
    },
    Compression::DXT1 | Compression::DXT2 | Compression::DXT3 | Compression::DXT4 | Compression::DXT5 => {
      Ok(decode_layers_dxt(&header, buf))
    },
    compression => Err(compression)
  }
}

fn pixels_into_bytes(pixels: Vec<[u8; 4]>) -> Vec<u8> {
  use std::mem::ManuallyDrop;
  unsafe {
    let mut pixels = ManuallyDrop::new(pixels);
    let ptr = pixels.as_mut_ptr() as *mut u8;
    let len = pixels.len() * 4;
    let cap = pixels.capacity() * 4;
    Vec::from_raw_parts(ptr, len, cap)
  }
}
