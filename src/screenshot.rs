#[derive(Debug)]
pub struct Screenshot {
    pub bgra: Vec<u8>,
    pub bounds: Rectangle<i32>,
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub struct Rectangle<T> {
    pub x: T,
    pub y: T,
    pub w: T,
    pub h: T,
}

use crate::bmp::{BMPHeader, InfoHeader};

use std::{io::Write, mem::size_of, ptr::null_mut};
use winapi::{
    ctypes::c_void,
    um::{
        wingdi::{
            BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits,
            SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, RGBQUAD, SRCCOPY,
        },
        winuser::{
            GetDC, GetSystemMetrics, ReleaseDC, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
            SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
        },
    },
};

impl Screenshot {
    pub fn take() -> Self {
        // get virtual screen bounds (covers all monitors)
        let x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
        let y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
        let w = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
        let h = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };

        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: h,
                biPlanes: 1,
                biBitCount: 24,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [RGBQUAD {
                rgbBlue: 0,
                rgbGreen: 0,
                rgbRed: 0,
                rgbReserved: 0,
            }],
        };

        let h_screen = unsafe { GetDC(null_mut()) };
        let h_dc = unsafe { CreateCompatibleDC(h_screen) };
        let h_bitmap = unsafe { CreateCompatibleBitmap(h_screen, w, h) };

        let mut bgra = Vec::with_capacity((w * h * 3) as usize);

        unsafe {
            let old_obj = SelectObject(h_dc, h_bitmap as *mut c_void);

            // Get pixels from the screen
            BitBlt(h_dc, 0, 0, w, h, h_screen, x, y, SRCCOPY);
            GetDIBits(
                h_screen,
                h_bitmap,
                0,
                h as u32,
                bgra.as_mut_ptr() as *mut c_void,
                &mut bi,
                DIB_RGB_COLORS,
            );

            bgra.set_len(bgra.capacity());
            SelectObject(h_dc, old_obj);
        }

        unsafe {
            DeleteObject(h_bitmap as *mut c_void);
            DeleteDC(h_dc);
            ReleaseDC(null_mut(), h_screen);
        }

        Screenshot {
            bounds: Rectangle { x, y, w, h },
            bgra,
        }
    }

    pub fn get_bmp_data(&self, region: Rectangle<u32>) -> Vec<u8> {
        let mut file: Vec<u8> = Vec::new();

        let row_size = (region.w * 3).next_multiple_of(4) as usize;
        let row_padding_nbytes = row_size - (region.w * 3) as usize;

        file.write_all(bytemuck::bytes_of(&BMPHeader::new(
            row_size * region.h as usize,
        )))
        .unwrap();
        file.write_all(bytemuck::bytes_of(&InfoHeader::new(
            region.w,
            region.h.wrapping_neg(),
        )))
        .unwrap();

        for h in 0..region.h {
            let row_start_byte = 3
                * (((self.bounds.h - 1) as u32 - region.y - h) * self.bounds.w as u32 + region.x)
                    as usize;
            let row_end_byte = row_start_byte + 3 * (region.w as usize);
            let row = &self.bgra[row_start_byte..row_end_byte];
            file.write_all(row).unwrap();
            let zeroes = [0u8; 3];
            file.write_all(&zeroes[0..row_padding_nbytes]).unwrap();
        }

        file
    }
}
