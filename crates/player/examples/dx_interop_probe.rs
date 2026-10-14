//! Windows GPU interop probe for docs/in-app-video.md phase 2.
//!
//! Checks which way an OpenGL context (which libmpv renders through) can share a texture with a
//! separate D3D11 device (standing in for GPUI's renderer), via WGL_NV_DX_interop2:
//!   A. GL renders into a shared D3D11 texture.
//!   B. GL renders into a shared D3D9Ex texture, which D3D11 opens by its share handle.
//! Each test clears the texture to a known color in GL and reads it back on the second device.
//!
//! ```sh
//! cargo build -p player --example dx_interop_probe --target x86_64-pc-windows-gnu
//! ```

#[cfg(not(windows))]
fn main() {
    eprintln!("this probe only runs on Windows");
}

#[cfg(windows)]
fn main() {
    if let Err(err) = probe::run() {
        eprintln!("probe failed: {err}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod probe {
    use std::ffi::{CStr, c_void};

    use windows::Win32::Foundation::*;
    use windows::Win32::Graphics::Direct3D::*;
    use windows::Win32::Graphics::Direct3D9::*;
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::Dxgi::Common::*;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::Graphics::OpenGL::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::core::{BOOL, Interface, PCSTR, w};

    type Error = Box<dyn std::error::Error>;

    const SIZE: u32 = 64;
    const GL_TEXTURE_2D: u32 = 0x0DE1;
    const GL_FRAMEBUFFER: u32 = 0x8D40;
    const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
    const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
    const WGL_ACCESS_WRITE_DISCARD_NV: u32 = 0x0002;
    /// The color GL clears to: r=1.0, g=0.0, b=0.5. Read back as BGRA bytes.
    const EXPECTED_BGRA: [u8; 4] = [128, 0, 255, 255];

    #[allow(non_snake_case)]
    struct Gl {
        wglDXSetResourceShareHandleNV: unsafe extern "system" fn(*mut c_void, HANDLE) -> BOOL,
        wglDXOpenDeviceNV: unsafe extern "system" fn(*mut c_void) -> HANDLE,
        wglDXCloseDeviceNV: unsafe extern "system" fn(HANDLE) -> BOOL,
        wglDXRegisterObjectNV: unsafe extern "system" fn(HANDLE, *mut c_void, u32, u32, u32) -> HANDLE,
        wglDXUnregisterObjectNV: unsafe extern "system" fn(HANDLE, HANDLE) -> BOOL,
        wglDXLockObjectsNV: unsafe extern "system" fn(HANDLE, i32, *mut HANDLE) -> BOOL,
        wglDXUnlockObjectsNV: unsafe extern "system" fn(HANDLE, i32, *mut HANDLE) -> BOOL,
        glGenFramebuffers: unsafe extern "system" fn(i32, *mut u32),
        glBindFramebuffer: unsafe extern "system" fn(u32, u32),
        glFramebufferTexture2D: unsafe extern "system" fn(u32, u32, u32, u32, i32),
        glCheckFramebufferStatus: unsafe extern "system" fn(u32) -> u32,
    }

    unsafe fn proc<T>(name: &str) -> Result<T, Error> {
        let cname = std::ffi::CString::new(name)?;
        let f = unsafe { wglGetProcAddress(PCSTR(cname.as_ptr() as *const u8)) }.ok_or(format!("{name} missing"))?;
        Ok(unsafe { std::mem::transmute_copy(&f) })
    }

    pub fn run() -> Result<(), Error> {
        unsafe {
            // A hidden window and a legacy WGL context are enough to query and use the extension.
            let instance = GetModuleHandleW(None)?;
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("probe"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                64,
                64,
                None,
                None,
                Some(instance.into()),
                None,
            )?;
            let hdc = GetDC(Some(hwnd));
            let pfd = PIXELFORMATDESCRIPTOR {
                nSize: size_of::<PIXELFORMATDESCRIPTOR>() as u16,
                nVersion: 1,
                dwFlags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
                iPixelType: PFD_TYPE_RGBA,
                cColorBits: 32,
                ..Default::default()
            };
            SetPixelFormat(hdc, ChoosePixelFormat(hdc, &pfd), &pfd)?;
            let glrc = wglCreateContext(hdc)?;
            wglMakeCurrent(hdc, glrc)?;

            let string = |name| CStr::from_ptr(glGetString(name) as *const i8).to_string_lossy().into_owned();
            println!("GL vendor:   {}", string(GL_VENDOR));
            println!("GL renderer: {}", string(GL_RENDERER));
            println!("GL version:  {}", string(GL_VERSION));

            let get_extensions: unsafe extern "system" fn(HDC) -> *const i8 = proc("wglGetExtensionsStringARB")?;
            let extensions = CStr::from_ptr(get_extensions(hdc)).to_string_lossy().into_owned();
            let interop2 = extensions.split(' ').any(|e| e == "WGL_NV_DX_interop2");
            let interop = extensions.split(' ').any(|e| e == "WGL_NV_DX_interop");
            println!("WGL_NV_DX_interop: {interop}, WGL_NV_DX_interop2: {interop2}");
            if !interop {
                return Err("WGL_NV_DX_interop isn't supported".into());
            }

            let gl = Gl {
                wglDXSetResourceShareHandleNV: proc("wglDXSetResourceShareHandleNV")?,
                wglDXOpenDeviceNV: proc("wglDXOpenDeviceNV")?,
                wglDXCloseDeviceNV: proc("wglDXCloseDeviceNV")?,
                wglDXRegisterObjectNV: proc("wglDXRegisterObjectNV")?,
                wglDXUnregisterObjectNV: proc("wglDXUnregisterObjectNV")?,
                wglDXLockObjectsNV: proc("wglDXLockObjectsNV")?,
                wglDXUnlockObjectsNV: proc("wglDXUnlockObjectsNV")?,
                glGenFramebuffers: proc("glGenFramebuffers")?,
                glBindFramebuffer: proc("glBindFramebuffer")?,
                glFramebufferTexture2D: proc("glFramebufferTexture2D")?,
                glCheckFramebufferStatus: proc("glCheckFramebufferStatus")?,
            };

            // The "GPUI" device that reads the result.
            let reader = d3d11_device()?;

            match test_d3d11(&gl, &reader) {
                Ok(pixel) => report("A. D3D11 shared texture", pixel),
                Err(err) => println!("A. D3D11 shared texture: FAILED: {err}"),
            }
            match test_d3d9ex(&gl, hwnd, &reader) {
                Ok(pixel) => report("B. D3D9Ex shared texture", pixel),
                Err(err) => println!("B. D3D9Ex shared texture: FAILED: {err}"),
            }
            Ok(())
        }
    }

    fn report(name: &str, pixel: [u8; 4]) {
        let verdict = if pixel == EXPECTED_BGRA { "OK" } else { "WRONG COLOR" };
        println!("{name}: {verdict} (read {pixel:?}, expected {EXPECTED_BGRA:?})");
    }

    unsafe fn d3d11_device() -> Result<ID3D11Device, Error> {
        let mut device = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )?;
        }
        Ok(device.ok_or("no D3D11 device")?)
    }

    /// Registers `object` with GL, clears it to the test color through an FBO, unregisters it.
    unsafe fn clear_through_gl(gl: &Gl, interop_device: HANDLE, object: *mut c_void) -> Result<(), Error> {
        unsafe {
            let mut texture = 0u32;
            glGenTextures(1, &mut texture);
            let handle = (gl.wglDXRegisterObjectNV)(interop_device, object, texture, GL_TEXTURE_2D, WGL_ACCESS_WRITE_DISCARD_NV);
            if handle.is_invalid() || handle.0.is_null() {
                return Err(format!("wglDXRegisterObjectNV failed: {:?}", windows::core::Error::from_thread()).into());
            }
            let mut objects = [handle];
            if !(gl.wglDXLockObjectsNV)(interop_device, 1, objects.as_mut_ptr()).as_bool() {
                return Err("wglDXLockObjectsNV failed".into());
            }
            let mut fbo = 0u32;
            (gl.glGenFramebuffers)(1, &mut fbo);
            (gl.glBindFramebuffer)(GL_FRAMEBUFFER, fbo);
            (gl.glFramebufferTexture2D)(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, texture, 0);
            let status = (gl.glCheckFramebufferStatus)(GL_FRAMEBUFFER);
            glViewport(0, 0, SIZE as i32, SIZE as i32);
            glClearColor(1.0, 0.0, 0.5, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            glFinish();
            (gl.glBindFramebuffer)(GL_FRAMEBUFFER, 0);
            let _ = (gl.wglDXUnlockObjectsNV)(interop_device, 1, objects.as_mut_ptr());
            let _ = (gl.wglDXUnregisterObjectNV)(interop_device, handle);
            if status != GL_FRAMEBUFFER_COMPLETE {
                return Err(format!("framebuffer incomplete: {status:#x}").into());
            }
            Ok(())
        }
    }

    /// Copies pixel (0, 0) of a shared texture opened on `reader` into a staging texture.
    unsafe fn read_pixel(reader: &ID3D11Device, shared: HANDLE) -> Result<[u8; 4], Error> {
        unsafe {
            let mut opened: Option<ID3D11Texture2D> = None;
            reader.OpenSharedResource(shared, &mut opened)?;
            let opened = opened.ok_or("OpenSharedResource returned nothing")?;
            let desc = D3D11_TEXTURE2D_DESC {
                Width: SIZE,
                Height: SIZE,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                ..Default::default()
            };
            let mut staging = None;
            reader.CreateTexture2D(&desc, None, Some(&mut staging))?;
            let staging = staging.ok_or("no staging texture")?;
            let context = reader.GetImmediateContext()?;
            context.CopyResource(&staging, &opened);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let bytes = std::slice::from_raw_parts(mapped.pData as *const u8, 4);
            let pixel = [bytes[0], bytes[1], bytes[2], bytes[3]];
            context.Unmap(&staging, 0);
            Ok(pixel)
        }
    }

    unsafe fn test_d3d11(gl: &Gl, reader: &ID3D11Device) -> Result<[u8; 4], Error> {
        unsafe {
            let writer = d3d11_device()?;
            let desc = D3D11_TEXTURE2D_DESC {
                Width: SIZE,
                Height: SIZE,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                MiscFlags: D3D11_RESOURCE_MISC_SHARED.0 as u32,
                ..Default::default()
            };
            let mut texture = None;
            writer.CreateTexture2D(&desc, None, Some(&mut texture))?;
            let texture: ID3D11Texture2D = texture.ok_or("no texture")?;
            let shared = texture.cast::<windows::Win32::Graphics::Dxgi::IDXGIResource>()?.GetSharedHandle()?;

            let interop_device = (gl.wglDXOpenDeviceNV)(writer.as_raw());
            if interop_device.0.is_null() {
                return Err("wglDXOpenDeviceNV(D3D11) failed".into());
            }
            let result = clear_through_gl(gl, interop_device, texture.as_raw());
            let _ = (gl.wglDXCloseDeviceNV)(interop_device);
            result?;
            writer.GetImmediateContext()?.Flush();
            read_pixel(reader, shared)
        }
    }

    unsafe fn test_d3d9ex(gl: &Gl, hwnd: HWND, reader: &ID3D11Device) -> Result<[u8; 4], Error> {
        unsafe {
            let d3d9 = Direct3DCreate9Ex(D3D_SDK_VERSION)?;
            let mut params = D3DPRESENT_PARAMETERS {
                Windowed: TRUE,
                SwapEffect: D3DSWAPEFFECT_DISCARD,
                hDeviceWindow: hwnd,
                BackBufferWidth: 1,
                BackBufferHeight: 1,
                BackBufferFormat: D3DFMT_UNKNOWN,
                ..Default::default()
            };
            let mut device = None;
            d3d9.CreateDeviceEx(
                D3DADAPTER_DEFAULT,
                D3DDEVTYPE_HAL,
                hwnd,
                (D3DCREATE_HARDWARE_VERTEXPROCESSING | D3DCREATE_MULTITHREADED | D3DCREATE_FPU_PRESERVE) as u32,
                &mut params,
                std::ptr::null_mut(),
                &mut device,
            )?;
            let device: IDirect3DDevice9Ex = device.ok_or("no D3D9Ex device")?;
            let mut texture = None;
            let mut shared = HANDLE::default();
            device.CreateTexture(
                SIZE,
                SIZE,
                1,
                D3DUSAGE_RENDERTARGET as u32,
                D3DFMT_A8R8G8B8,
                D3DPOOL_DEFAULT,
                &mut texture,
                &mut shared,
            )?;
            let texture: IDirect3DTexture9 = texture.ok_or("no D3D9 texture")?;

            let interop_device = (gl.wglDXOpenDeviceNV)(device.as_raw());
            if interop_device.0.is_null() {
                return Err("wglDXOpenDeviceNV(D3D9Ex) failed".into());
            }
            let _ = (gl.wglDXSetResourceShareHandleNV)(texture.as_raw(), shared);
            let result = clear_through_gl(gl, interop_device, texture.as_raw());
            let _ = (gl.wglDXCloseDeviceNV)(interop_device);
            result?;
            read_pixel(reader, shared)
        }
    }
}
