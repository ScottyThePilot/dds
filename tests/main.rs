extern crate dds;
extern crate image;

use std::fs::File;
use std::io::BufReader;

use dds::Dds;
use image::{Rgba, RgbaImage};

fn compare_dds_to_png(dds_path: String, png_path: String) {
  let mut reader = BufReader::new(File::open(dds_path).unwrap());
  let dds = Dds::decode(&mut reader).unwrap();

  let img = image::open(png_path).unwrap();
  let img = img.into_rgba8();

  for (x, y, Rgba(pixel)) in img.enumerate_pixels() {
    let Rgba(other_pixel) = dds.layers[0].get_pixel(x, y);
    assert_eq!(pixel, other_pixel);
  }

  assert_eq!(img, dds.layers[0]);
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_encode_uncompressed() {
    let image = RgbaImage::from_raw(4, 4, vec![0u8; 64]).unwrap();

    let mut bytes = Vec::new();
    Dds::encode_uncompressed(&mut bytes, &image).unwrap();

    let dds = Dds::decode(bytes.as_slice()).unwrap();

    assert_eq!(dds.layers.len(), 1);
    assert_eq!(image, dds.layers[0]);
  }

  #[test]
  fn test_encode_uncompressed_rectangular() {
    let image = RgbaImage::from_raw(8, 4, (0u8..128).collect()).unwrap();

    let mut bytes = Vec::new();
    Dds::encode_uncompressed(&mut bytes, &image).unwrap();

    let dds = Dds::decode(bytes.as_slice()).unwrap();

    assert_eq!(dds.layers.len(), 1);
    assert_eq!(image, dds.layers[0]);
  }

  #[test]
  fn test_dds_vs_png() {
    let filenames = [
      "dxt1",
      "dxt5",
      "qt/DXT1",
      "qt/DXT2",
      "qt/DXT3",
      "qt/DXT4",
      "qt/DXT5",
      "qt/A8R8G8B8",
      "qt/A8R8G8B8.2"
    ];

    for filename in filenames.iter() {
      compare_dds_to_png(
        format!("./samples/{}.dds", filename),
        format!("./samples/{}.png", filename)
      );
    }
  }
}
