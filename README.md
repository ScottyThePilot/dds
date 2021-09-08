# DDS

This library is a fork of https://gitlab.com/mechaxl/dds-rs, it attempts to make the API easier to use and more flexible.
This library supports decoding of uncompressed DDS files and DXT1-5 files. Supports encoding in the A8R8G8B8 format.

## Example
```rust
extern crate dds;

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use dds::Dds;

fn main() {
  let file = File::open(Path::new("./samples/dxt1.dds")).unwrap();
  let reader = BufReader::new(file);

  let dds = Dds::decode(reader).unwrap();

  for layer_image in &dds.layers[..] {
    // `layer_image` is an `image::RgbaImage`
    println!("{:?}", layer_image.dimensions());
  };
}
```
