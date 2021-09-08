//! Handles decoding and basic encoding of DirectDraw Surface files.
//!
//! # Example
//!
//! ```rust
//! extern crate dds;
//!
//! use std::fs::File;
//! use std::io::BufReader;
//! use std::path::Path;
//!
//! use dds::Dds;
//!
//! fn main() {
//!   let file = File::open(Path::new("./samples/dxt1.dds")).unwrap();
//!   let reader = BufReader::new(file);
//!
//!   let dds = Dds::decode(reader).unwrap();
//!
//!   for layer_image in &dds.layers[..] {
//!     println!("{:?}", layer_image.dimensions());
//!   };
//! }
//! ```

extern crate bincode;
extern crate image;
extern crate serde;
extern crate thiserror;

mod format;

use bincode::ErrorKind as BincodeError;
use image::RgbaImage;
use serde::{Serialize, Deserialize};
use thiserror::Error;

use crate::format::decode_layers;

use std::fmt;
use std::io::{self, Read, Write};

/// Represents an error encountered while decoding/parsing a DDS file.
#[derive(Debug, Error)]
pub enum DecodeError {
  #[error(transparent)]
  Io(#[from] io::Error),
  #[error(transparent)]
  DecodeHeader(#[from] Box<BincodeError>),
  #[error("unexpected end of file while reading magic bytes or header")]
  UnexpectedEOF,
  #[error("expected the file to start with `DDS `, got `{}` instead", String::from_utf8_lossy(.0))]
  InvalidMagicBytes([u8; 4]),
  #[error("compression mode {0} is unsupported")]
  UnsupportedCompression(Compression)
}

/// Represents an error encountered while encoding.
#[derive(Debug, Error)]
pub enum EncodeError {
  #[error(transparent)]
  Io(#[from] io::Error),
  #[error(transparent)]
  EncodeHeader(#[from] Box<BincodeError>),
  #[error("compression mode {0} is unsupported")]
  UnsupportedCompression(Compression)
}

/// Pixel information as represented in the DDS file
///
/// Direct translation of struct found here:
/// <https://msdn.microsoft.com/en-us/library/bb943984.aspx>
#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawPixelFormat {
  pub size: u32,
  pub flags: u32,
  pub four_cc: [u8; 4],
  pub rgb_bit_count: u32,
  pub red_bit_mask: u32,
  pub green_bit_mask: u32,
  pub blue_bit_mask: u32,
  pub alpha_bit_mask: u32
}

impl RawPixelFormat {
  // Parses some common pixel formats from the raw bit masks, for convenience
  fn to_pixel_format(&self) -> PixelFormat {
    let RawPixelFormat {
      rgb_bit_count: count,
      red_bit_mask: r,
      green_bit_mask: g,
      blue_bit_mask: b,
      alpha_bit_mask: a,
      ..
    } = self;

    match (count, r, g, b, a) {
      (16, 0x7C00, 0x3E0, 0x1F, 0x8000) => PixelFormat::A1R5G5B5,
      (32, 0x3FF, 0xFFC00, 0x3FF00000, 0xC0000000) => PixelFormat::A2B10G10R10,
      (32, 0x3FF00000, 0xFFC00, 0x3FF, 0xC0000000) => PixelFormat::A2R10G10B10,
      (8, 0xF, 0x0, 0x0, 0xF0) => PixelFormat::A4L4,
      (16, 0xF00, 0xF0, 0xF, 0xF000) => PixelFormat::A4R4G4B4,
      (8, 0x0, 0x0, 0x0, 0xFF) => PixelFormat::A8,
      (32, 0xFF, 0xFF00, 0xFF0000, 0xFF000000) => PixelFormat::A8B8G8R8,
      (16, 0xFF, 0x0, 0x0, 0xFF00) => PixelFormat::A8L8,
      (16, 0xE0, 0x1C, 0x3, 0xFF00) => PixelFormat::A8R3G3B2,
      (32, 0xFF0000, 0xFF00, 0xFF, 0xFF000000) => PixelFormat::A8R8G8B8,
      (32, 0xFFFF, 0xFFFF0000, 0x0, 0x0) => PixelFormat::G16R16,
      (16, 0xFFFF, 0x0, 0x0, 0x0) => PixelFormat::L16,
      (8, 0xFF, 0x0, 0x0, 0x0) => PixelFormat::L8,
      (16, 0xF800, 0x7E0, 0x1F, 0x0) => PixelFormat::R5G6B5,
      (24, 0xFF0000, 0xFF00, 0xFF, 0x0) => PixelFormat::R8G8B8,
      (16, 0x7C00, 0x3E0, 0x1F, 0x0) => PixelFormat::X1R5G5B5,
      (16, 0xF00, 0xF0, 0xF, 0x0) => PixelFormat::X4R4G4B4,
      (32, 0xFF, 0xFF00, 0xFF0000, 0x0) => PixelFormat::X8B8G8R8,
      (32, 0xFF0000, 0xFF00, 0xFF, 0x0) => PixelFormat::X8R8G8B8,
      (_, _, _, _, _) => PixelFormat::Unknown
    }
  }
}

/// Header as represented in the DDS file
///
/// Direct translation of struct found here:
/// <https://msdn.microsoft.com/en-us/library/bb943982.aspx>
#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawHeader {
  pub size: u32,
  pub flags: u32,
  pub height: u32,
  pub width: u32,
  pub pitch_or_linear_size: u32,
  pub depth: u32,
  pub mipmap_count: u32,
  pub reserved: [u32; 11],
  pub pixel_format: RawPixelFormat,
  pub caps: u32,
  pub caps2: u32,
  pub caps3: u32,
  pub caps4: u32,
  pub reserved2: u32
}

impl RawHeader {
  const fn new_uncompressed(height: u32, width: u32) -> RawHeader {
    RawHeader {
      size: height * width * 4,
      flags: 0,
      height,
      width,
      pitch_or_linear_size: 0,
      depth: 0,
      mipmap_count: 0,
      reserved: [0; 11],
      pixel_format: RawPixelFormat {
        size: 0,
        flags: 0x41,
        four_cc: [0; 4],
        rgb_bit_count: 32,
        red_bit_mask: 0xFF,
        green_bit_mask: 0xFF00,
        blue_bit_mask: 0xFF0000,
        alpha_bit_mask: 0xFF000000
      },
      caps: 0,
      caps2: 0,
      caps3: 0,
      caps4: 0,
      reserved2: 0
    }
  }

  /// Parses the raw header from the image. Useful for getting information not contained
  /// in the normal parsed Header struct.
  pub fn decode<R: Read>(mut reader: R) -> Result<RawHeader, DecodeError> {
    let mut header_buf = [0u8; 124];

    let mut magic_bytes_buf = [0; 4];
    reader.read_exact(&mut magic_bytes_buf)?;

    // If the file doesn't start with `DDS `, abort decoding
    if &magic_bytes_buf != b"DDS " {
      return Err(DecodeError::InvalidMagicBytes(magic_bytes_buf));
    };

    reader.read_exact(&mut header_buf)?;

    Ok(bincode::deserialize(&header_buf)?)
  }

  pub fn encode<W: Write>(&self, mut writer: W) -> Result<(), EncodeError> {
    writer.write_all(b"DDS ")?;
    bincode::serialize_into(writer, self)
      .map_err(From::from)
  }
}

/// Convenience enum for storing common pixel formats
///
/// See here for more information about the common formats:
/// <https://msdn.microsoft.com/en-us/library/bb943991.aspx>
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PixelFormat {
  A1R5G5B5,
  A2B10G10R10,
  A2R10G10B10,
  A4L4,
  A4R4G4B4,
  A8,
  A8B8G8R8,
  A8L8,
  A8R3G3B2,
  A8R8G8B8,
  G16R16,
  L16,
  L8,
  R5G6B5,
  R8G8B8,
  Unknown,
  X1R5G5B5,
  X4R4G4B4,
  X8B8G8R8,
  X8R8G8B8
}

impl fmt::Display for PixelFormat {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "{:?}", self)
  }
}

/// Represents the compression format of a DDS file, aka the four-cc bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Compression {
  DXT1,
  DXT2,
  DXT3,
  DXT4,
  DXT5,
  DX10,
  None,
  Other([u8; 4])
}

impl Compression {
  pub fn from_bytes(bytes: [u8; 4]) -> Compression {
    match &bytes {
      &[0, 0, 0, 0] => Compression::None,
      b"DXT1" => Compression::DXT1,
      b"DXT2" => Compression::DXT2,
      b"DXT3" => Compression::DXT3,
      b"DXT4" => Compression::DXT4,
      b"DXT5" => Compression::DXT5,
      b"DX10" => Compression::DX10,
      _ => Compression::Other(bytes)
    }
  }

  pub fn to_bytes(self) -> [u8; 4] {
    match self {
      Compression::DXT1 => *b"DXT1",
      Compression::DXT2 => *b"DXT2",
      Compression::DXT3 => *b"DXT3",
      Compression::DXT4 => *b"DXT4",
      Compression::DXT5 => *b"DXT5",
      Compression::DX10 => *b"DX10",
      Compression::None => [0; 4],
      Compression::Other(bytes) => bytes
    }
  }
}

impl fmt::Display for Compression {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match self {
      Compression::DXT1 => write!(f, "DXT1"),
      Compression::DXT2 => write!(f, "DXT2"),
      Compression::DXT3 => write!(f, "DXT3"),
      Compression::DXT4 => write!(f, "DXT4"),
      Compression::DXT5 => write!(f, "DXT5"),
      Compression::DX10 => write!(f, "DX10"),
      Compression::None => write!(f, "None"),
      Compression::Other(bytes) => write!(f, "{}", String::from_utf8_lossy(bytes))
    }
  }
}

/// Represents a parsed DDS header. Has several convenience attributes.
#[derive(Debug, Clone, PartialEq, Eq,)]
pub struct Header {
  /// Height of the main image
  pub height: u32,
  /// Width of the main image
  pub width: u32,
  /// How many levels of mipmaps there are
  pub mipmap_count: u32,
  /// Compression type used
  pub compression: Compression,
  /// The 4-character code for this image
  pub fourcc: [u8; 4],
  /// The pixel format used
  pub pixel_format: PixelFormat,
  /// The number of bytes used per-pixel
  pub pixel_bytes: usize,
  /// The bit masks used for each channel
  pub channel_masks: [u32; 4]
}

impl Header {
  /// Parses a `Header` object from a reader.
  pub fn decode<R: Read>(reader: R) -> Result<Header, DecodeError> {
    let raw_header = RawHeader::decode(reader)?;

    Ok(Header {
      height: raw_header.height,
      width: raw_header.width,
      mipmap_count: raw_header.mipmap_count,
      compression: Compression::from_bytes(raw_header.pixel_format.four_cc),
      fourcc: raw_header.pixel_format.four_cc,
      pixel_format: raw_header.pixel_format.to_pixel_format(),
      pixel_bytes: raw_header.pixel_format.rgb_bit_count as usize / 8,
      channel_masks: [
        raw_header.pixel_format.red_bit_mask,
        raw_header.pixel_format.green_bit_mask,
        raw_header.pixel_format.blue_bit_mask,
        raw_header.pixel_format.alpha_bit_mask
      ]
    })
  }

  // Returns layer sizes
  fn get_layer_sizes(&self) -> Vec<(usize, usize)> {
    // Files with only a single texture will often have
    // the mipmap count set to 0, so we force generating
    // at least a single level
    let count: u32 = self.mipmap_count.max(1);
    let mut layers = Vec::with_capacity(count as usize);
    for i in 0..count {
      layers.push(((self.height / 2u32.pow(i)) as usize, (self.width / 2u32.pow(i)) as usize));
    };

    layers
  }
}

/// Represents a parsed DDS file
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dds {
  /// The parsed DDS header
  pub header: Header,
  /// Mipmap layers
  pub layers: Vec<RgbaImage>
}

impl Dds {
  /// Decodes a buffer into a header and a series of mipmap images.
  /// Handles uncompressed and DXT1-5 compressed images.
  pub fn decode<R: Read>(mut reader: R) -> Result<Dds, DecodeError> {
    let header = Header::decode(&mut reader)?;

    let mut buf = Vec::new();
    reader.read_to_end(&mut buf)?;

    let layers = decode_layers(&header, &buf)
      .map_err(DecodeError::UnsupportedCompression)?;

    Ok(Dds { header, layers })
  }

  /// Encodes an RGBA image as an uncompressed A8R8G8B8 DDS.
  pub fn encode_uncompressed<W: Write>(mut writer: W, image: &RgbaImage) -> Result<(), EncodeError> {
    let (width, height) = image.dimensions();
    RawHeader::new_uncompressed(height, width).encode(&mut writer)?;

    let data: &[u8] = image.as_raw();
    writer.write_all(data)?;

    Ok(())
  }

  /// Encodes a series of Pixels as a bunch of bytes, suitable for writing to disk, etc.
  /// Currently only supports uncompressed RGBA images.
  pub fn encode<W: Write>(writer: W, image: &RgbaImage, compression: Compression) -> Result<(), EncodeError> {
    match compression {
      Compression::None => Dds::encode_uncompressed(writer, image),
      compression => Err(EncodeError::UnsupportedCompression(compression))
    }
  }
}
