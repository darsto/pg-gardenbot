use bytemuck::{Pod, Zeroable};

#[derive(Copy, Clone, Pod, Zeroable)]
#[repr(C, packed)]
pub struct BMPHeader {
    /// always "BM"
    magic: [u8; 2],
    /// size of the whole file in bytes
    size: u32,
    reserved1: u16,
    reserved2: u16,
    /// offset to pixel data starting from the beginning of the file
    data_offset: u32,
}

impl BMPHeader {
    pub fn new(data_size: usize) -> Self {
        let data_offset =
            (core::mem::size_of::<InfoHeader>() + core::mem::size_of::<BMPHeader>()) as u32; // TODO: + color table if set
        let size = data_size as u32 + data_offset;
        Self {
            magic: [b'B', b'M'],
            size,
            reserved1: 0,
            reserved2: 0,
            data_offset,
        }
    }
}

#[derive(Copy, Clone, Pod, Zeroable)]
#[repr(C, packed)]
pub struct InfoHeader {
    /// size of this structure in bytes
    size: u32,
    /// width in pixels
    width: u32,
    /// height in pixels
    height: u32,
    /// number of planes, you most likely want just 1
    planes: u16,
    /// bits per pixel (1, 4, 8, 24, 32)
    /// 32-bit bitmaps (with alpha) can't be processed by most software
    bit_depth: u16,
    /// type of compression, usually zero for no compression
    compression: u32,
    /// size of uncompressed data. If there's no compression, this can be 0
    raw_size: u32,
    /// horizontal resolution for the target device. Usually unused and 0
    x_pixels_per_meter: u32,
    /// vertical resolution for the target device. Usually unused and 0
    y_pixels_per_meter: u32,
    /// number of entries in the color table. Usually unused and 0
    color_table_size: u32,
    /// minimum number of entries in the color table to display this bitmap.
    /// Usually unused and 0
    color_table_min_size: u32,
}

impl InfoHeader {
    pub fn new(width: u32, height: u32) -> InfoHeader {
        InfoHeader {
            size: core::mem::size_of::<InfoHeader>() as u32,
            width,
            height,
            bit_depth: 24, // TODO: un-hardcode
            planes: 1,
            compression: 0,
            raw_size: 0,
            x_pixels_per_meter: 0,
            y_pixels_per_meter: 0,
            color_table_size: 0, // TODO: support limited-color bmp (256bit, 16bit, 2bit)
            color_table_min_size: 0,
        }
    }
}
