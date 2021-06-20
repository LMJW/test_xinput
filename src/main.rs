use bindings::Windows::Win32::Foundation::*;
use bindings::Windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateCompatibleDC, CreateDIBSection, EndPaint, FillRect, PatBlt, StretchDIBits,
    HBRUSH, PAINTSTRUCT, *,
};
use bindings::Windows::Win32::System::Diagnostics::Debug::{GetLastError, *};
use bindings::Windows::Win32::System::LibraryLoader::GetModuleHandleA;
use bindings::Windows::Win32::System::Memory::{VirtualAlloc, VirtualFree, *};
use bindings::Windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExA, DefWindowProcA, DispatchMessageA, GetClientRect, RegisterClassA, ShowWindow,
    TranslateMessage, CW_USEDEFAULT, WNDCLASSA, WNDPROC, *,
};
use bindings::Windows::Win32::UI::XInput::{XInputGetState, *};

use std::ffi::{c_void, CString};
struct Win32OffscreenBuffer {
    info: BITMAPINFO,
    memory: *mut c_void,
    width: i32,
    height: i32,
    bytes_per_pixel: i32,
}

// using a static global variable to control the close of window can be more
// responsive than `PostQuitMessage` as the latter can be lagging sometimes.
static mut RUNNING: bool = false;

static mut GLOBAL_BACK_BUFFER: Option<Win32OffscreenBuffer> = None;

impl Default for Win32OffscreenBuffer {
    fn default() -> Self {
        Self {
            info: BITMAPINFO::default(),
            memory: 0 as *mut _,
            width: 0,
            height: 0,
            bytes_per_pixel: 4,
        }
    }
}

fn win32_resize_dib_section(
    screen_buffer: &mut Option<Win32OffscreenBuffer>,
    width: i32,
    height: i32,
) {
    unsafe {
        if screen_buffer.is_some() {
            let buffer = screen_buffer.as_mut().unwrap();
            VirtualFree(buffer.memory, 0, MEM_RELEASE);
        } else {
            screen_buffer.replace(Win32OffscreenBuffer::default());
        }
    }

    let mut info = BITMAPINFO::default();
    info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;

    // negative hight is to tell windows to use the top left as orgin
    info.bmiHeader.biHeight = -height;
    info.bmiHeader.biWidth = width;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB as u32;

    unsafe {
        let mut buffer = screen_buffer.as_mut().unwrap();
        buffer.info = info;
        buffer.width = width;
        buffer.height = height;

        let bitmap_mem_size = (width * height * buffer.bytes_per_pixel) as usize;
        buffer.memory = VirtualAlloc(0 as *mut _, bitmap_mem_size, MEM_COMMIT, PAGE_READWRITE);
    }
}

fn win32_copy_buffer_to_window(
    device_ctx: HDC,
    screen_buffer: &Win32OffscreenBuffer,
    window_width: i32,
    window_height: i32,
) {
    unsafe {
        StretchDIBits(
            device_ctx,
            0,
            0,
            window_width,
            window_height,
            0,
            0,
            screen_buffer.width,
            screen_buffer.height,
            screen_buffer.memory,
            &screen_buffer.info,
            DIB_RGB_COLORS,
            SRCCOPY,
        )
    };
}

fn win32_get_window_dimension(hwnd: HWND) -> (i32, i32) {
    let mut rect = RECT::default();

    unsafe { GetClientRect(hwnd, &mut rect) };
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;

    (width, height)
}

unsafe extern "system" fn wndproc_callback(
    hwnd: HWND,
    umsg: u32,
    wpram: WPARAM,
    lpram: LPARAM,
) -> LRESULT {
    match umsg {
        WM_CLOSE => {
            RUNNING = false;
            LRESULT(0)
            // Potential BUG??? 
            //
            // The current form of code will have STATUS_ACCESS_VIOLATION error
            // when close the window. The panic of the window seems due to the
            // use of XInputGetState. When close the window, if WM_CLOSE return
            // LRESULT(0), the window will try to access the null pointer and
            // causing it to return the error blow. If I replace the return
            // value `LRESULT(0)` to `DefWindowProcA`, then the error will not
            // appear.
            //
            // error: process didn't exit successfully:
            // `target\debug\XInputGetStateBug.exe` (exit code: 0xc0000005,
            // STATUS_ACCESS_VIOLATION)

            // using the blow line as return will works okay.
            
            // DefWindowProcA(hwnd, umsg, wpram, lpram)
        }

        WM_PAINT => {
            println!("WM_PAINT");
            let mut paint = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut paint);

            let (wind_w, wind_h) = win32_get_window_dimension(hwnd);
            win32_copy_buffer_to_window(hdc, &GLOBAL_BACK_BUFFER.as_ref().unwrap(), wind_w, wind_h);

            EndPaint(hwnd, &paint);
            LRESULT(0)

        }
        _ => DefWindowProcA(hwnd, umsg, wpram, lpram),
    }
}

fn main() -> windows::Result<()> {
    let class_name = "testClass";
    let c_class_name = CString::new(class_name).expect("lpszClassName");

    unsafe {
        win32_resize_dib_section(&mut GLOBAL_BACK_BUFFER, 1280, 720);
    }

    let mut window_class = WNDCLASSA::default();
    window_class.lpfnWndProc = Some(wndproc_callback);
    window_class.hInstance = unsafe { GetModuleHandleA(None) };
    window_class.lpszClassName = PSTR(c_class_name.as_ptr() as *mut _);
    // if we don't have this, the re-draw won't happen when we re-draw the window
    // thus we will need to have these two flags
    window_class.style = CS_HREDRAW | CS_VREDRAW;

    unsafe {
        RegisterClassA(&window_class);

        // Successfully registered the window
        let hwnd = CreateWindowExA(
            Default::default(),
            class_name,
            "Test Class",
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            None,
            None,
            GetModuleHandleA(None),
            0 as *mut _,
        );
        if hwnd.is_null() {
            println!("hwnd is null");
            return Ok(());
        } else {
            RUNNING = true;
            ShowWindow(hwnd, SW_SHOW);

            let mut msg = MSG::default();

            while RUNNING {
                while PeekMessageA(&mut msg, hwnd, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        RUNNING = false;
                    }

                    TranslateMessage(&msg);
                    DispatchMessageA(&msg);
                }

                // get user input here
                for controller_index in 0..XUSER_MAX_COUNT {
                    let mut controller_state = XINPUT_STATE::default();
                    XInputGetState(controller_index, &mut controller_state);
                }
            }
        }
    }

    Ok(())
}
