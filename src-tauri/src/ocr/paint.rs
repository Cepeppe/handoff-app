//! Text drawn with GDI, so that the OCR tests read back something a real recogniser was
//! trained on.
//!
//! A hand-drawn bitmap font would make the assertions a test of our own glyphs rather than
//! of the engine; a committed PNG would be a blob nobody can regenerate. This renders with
//! a font every Windows install has, at a size a dashboard would use, and the pixels never
//! leave the test process.
//!
//! It is compiled only into the test binaries and only on Windows: the `Win32_Graphics_Gdi`
//! feature it needs is a **dev**-dependency, so nothing the application ships can reach it.
//! Both engines' suites read from here — the Windows engine's and the bundled engine's —
//! which is why it is a file of its own rather than a module inside one of them. The
//! cross-platform renderer that would let the macOS leg run the same assertions is T-048's
//! corpus generator.

use windows::core::HSTRING;
use windows::Win32::Foundation::COLORREF;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, PatBlt,
    SelectObject, SetBkMode, SetTextColor, TextOutW, BITMAPINFO, BI_RGB, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS, FF_SWISS, FW_NORMAL,
    HGDIOBJ, OUT_TT_PRECIS, TRANSPARENT, WHITENESS,
};

use image::{Rgba, RgbaImage};

/// A white image with `lines` drawn in black, one under the other.
pub fn text(width: u32, height: u32, point_size: i32, lines: &[&str]) -> RgbaImage {
    let mut header = BITMAPINFO::default();
    header.bmiHeader.biSize =
        u32::try_from(size_of::<windows::Win32::Graphics::Gdi::BITMAPINFOHEADER>())
            .expect("the header fits in a u32");
    header.bmiHeader.biWidth = i32::try_from(width).expect("a sane width");
    // Negative: a top-down DIB, so row 0 of the buffer is row 0 of the image.
    header.bmiHeader.biHeight = -i32::try_from(height).expect("a sane height");
    header.bmiHeader.biPlanes = 1;
    header.bmiHeader.biBitCount = 32;
    header.bmiHeader.biCompression = BI_RGB.0;

    // SAFETY: every handle created here is selected out and deleted before the function
    // returns, and `bits` is read only while the bitmap is alive and only for the
    // `width * height * 4` bytes `CreateDIBSection` promised.
    unsafe {
        let dc = CreateCompatibleDC(None);
        assert!(!dc.is_invalid(), "a memory device context");
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bitmap = CreateDIBSection(Some(dc), &header, DIB_RGB_COLORS, &mut bits, None, 0)
            .expect("a device-independent bitmap");
        let previous_bitmap = SelectObject(dc, HGDIOBJ::from(bitmap));

        let _ = PatBlt(
            dc,
            0,
            0,
            i32::try_from(width).expect("a sane width"),
            i32::try_from(height).expect("a sane height"),
            WHITENESS,
        );

        let font = CreateFontW(
            -point_size,
            0,
            0,
            0,
            i32::try_from(FW_NORMAL.0).expect("a weight"),
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_TT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            u32::from(DEFAULT_PITCH.0 | FF_SWISS.0),
            &HSTRING::from("Segoe UI"),
        );
        let previous_font = SelectObject(dc, HGDIOBJ::from(font));
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x0000_0000));

        for (index, line) in lines.iter().enumerate() {
            let wide: Vec<u16> = line.encode_utf16().collect();
            let y =
                8 + i32::try_from(index).expect("a line number") * (point_size + point_size / 3);
            let _ = TextOutW(dc, 8, y, &wide);
        }

        let mut image = RgbaImage::new(width, height);
        let source = std::slice::from_raw_parts(bits.cast::<u8>(), (width * height * 4) as usize);
        for (index, pixel) in image.pixels_mut().enumerate() {
            let at = index * 4;
            *pixel = Rgba([source[at + 2], source[at + 1], source[at], 0xff]);
        }

        SelectObject(dc, previous_font);
        SelectObject(dc, previous_bitmap);
        let _ = DeleteObject(HGDIOBJ::from(font));
        let _ = DeleteObject(HGDIOBJ::from(bitmap));
        let _ = DeleteDC(dc);
        image
    }
}
