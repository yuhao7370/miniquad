//! Desktop OpenGL on an EGL pbuffer, independent of X11 and Wayland.

use super::{egl::*, gl, module::Module};
use crate::{conf::Conf, EventHandler};
use std::{
    ffi::{c_void, CStr, CString},
    ptr::{null, null_mut},
};

const EGL_EXTENSIONS: EGLint = 0x3055;
const EGL_PLATFORM_SURFACELESS_MESA: u32 = 0x31DD;
const EGL_PLATFORM_DEVICE_EXT: u32 = 0x313F;
const EGL_OPENGL_API: u32 = 0x30A2;
const EGL_RENDERABLE_TYPE: EGLint = 0x3040;
const EGL_OPENGL_BIT: EGLint = 0x0008;
const EGL_PBUFFER_BIT: EGLint = 0x0001;

type GlProc = unsafe extern "C" fn();
type BindApi = unsafe extern "C" fn(u32) -> EGLBoolean;
type QueryApi = unsafe extern "C" fn() -> u32;
type QueryDevices = unsafe extern "C" fn(EGLint, *mut *mut c_void, *mut EGLint) -> EGLBoolean;

fn error(egl: &LibEgl, operation: &str) -> String {
    format!("{operation} failed (EGL error {:#x})", unsafe {
        (egl.eglGetError)()
    })
}

struct ClientApi {
    bind: BindApi,
    previous: u32,
}

impl ClientApi {
    unsafe fn bind_gl(egl: &LibEgl) -> Result<Self, String> {
        let bind: BindApi = egl
            .module()
            .get_symbol("eglBindAPI")
            .map_err(|e| e.to_string())?;
        let query: QueryApi = egl
            .module()
            .get_symbol("eglQueryAPI")
            .map_err(|e| e.to_string())?;
        let previous = query();
        if bind(EGL_OPENGL_API) == 0 {
            return Err(error(egl, "eglBindAPI(OpenGL)"));
        }
        Ok(Self { bind, previous })
    }
}

impl Drop for ClientApi {
    fn drop(&mut self) {
        unsafe { (self.bind)(self.previous) };
    }
}

// EGL 1.5 and EGL_EXT_platform_base use different attribute list types.
unsafe fn platform_display(
    egl: &LibEgl,
    platform: u32,
    device: *mut c_void,
) -> Result<EGLDisplay, String> {
    type Core = unsafe extern "C" fn(u32, *mut c_void, *const isize) -> EGLDisplay;
    type Ext = unsafe extern "C" fn(u32, *mut c_void, *const EGLint) -> EGLDisplay;
    if let Ok(get) = egl.module().get_symbol::<Core>("eglGetPlatformDisplay") {
        return Ok(get(platform, device, null()));
    }
    let proc = (egl.eglGetProcAddress)(b"eglGetPlatformDisplayEXT\0".as_ptr().cast())
        .ok_or("EGL has no platform display entry point")?;
    let get: Ext = std::mem::transmute::<GlProc, Ext>(proc);
    Ok(get(platform, device, null()))
}

struct Context<'a> {
    egl: &'a LibEgl,
    display: EGLDisplay,
    surface: EGLSurface,
    context: EGLContext,
}

impl<'a> Context<'a> {
    unsafe fn create(egl: &'a LibEgl, platform: u32, device: *mut c_void) -> Result<Self, String> {
        let display = platform_display(egl, platform, device)?;
        if display.is_null() || (egl.eglInitialize)(display, null_mut(), null_mut()) == 0 {
            return Err(error(egl, "initializing the headless EGL display"));
        }
        let mut ctx = Self {
            egl,
            display,
            surface: null_mut(),
            context: null_mut(),
        };
        #[rustfmt::skip]
        let attrs = [
            EGL_SURFACE_TYPE as EGLint, EGL_PBUFFER_BIT,
            EGL_RENDERABLE_TYPE, EGL_OPENGL_BIT,
            EGL_RED_SIZE as EGLint, 8, EGL_GREEN_SIZE as EGLint, 8,
            EGL_BLUE_SIZE as EGLint, 8, EGL_ALPHA_SIZE as EGLint, 8,
            EGL_NONE as EGLint,
        ];
        let mut config = null_mut();
        let mut count = 0;
        if (egl.eglChooseConfig)(display, attrs.as_ptr(), &mut config, 1, &mut count) == 0 {
            return Err(error(egl, "eglChooseConfig"));
        }
        if count == 0 {
            return Err("no RGBA8 desktop OpenGL pbuffer configuration".into());
        }
        let attrs = [EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE];
        ctx.context = (egl.eglCreateContext)(display, config, null_mut(), attrs.as_ptr().cast());
        if ctx.context.is_null() {
            return Err(error(egl, "eglCreateContext(OpenGL 3)"));
        }
        // The requested render dimensions belong to the shared offscreen framebuffer.
        let attrs = [EGL_WIDTH, 1, EGL_HEIGHT, 1, EGL_NONE];
        ctx.surface = (egl.eglCreatePbufferSurface)(display, config, attrs.as_ptr().cast());
        if ctx.surface.is_null() {
            return Err(error(egl, "eglCreatePbufferSurface"));
        }
        if (egl.eglMakeCurrent)(display, ctx.surface, ctx.surface, ctx.context) == 0 {
            return Err(error(egl, "eglMakeCurrent"));
        }
        Ok(ctx)
    }

    unsafe fn open(egl: &'a LibEgl) -> Result<Self, String> {
        let extensions = (egl.eglQueryString)(null_mut(), EGL_EXTENSIONS);
        if extensions.is_null() {
            return Err(error(egl, "querying EGL client extensions"));
        }
        let extensions = CStr::from_ptr(extensions).to_bytes();
        let supports = |name: &[u8]| extensions.split(|c| *c == b' ').any(|ext| ext == name);
        let mut last_error = "EGL has no supported headless display platform".to_string();
        if supports(b"EGL_MESA_platform_surfaceless") {
            match Self::create(egl, EGL_PLATFORM_SURFACELESS_MESA, null_mut()) {
                Ok(ctx) => return Ok(ctx),
                Err(e) => last_error = e,
            }
        }
        if supports(b"EGL_EXT_platform_device") {
            let proc = (egl.eglGetProcAddress)(b"eglQueryDevicesEXT\0".as_ptr().cast())
                .ok_or("EGL does not expose eglQueryDevicesEXT")?;
            let query = std::mem::transmute::<GlProc, QueryDevices>(proc);
            let mut count = 0;
            if query(0, null_mut(), &mut count) != 0 && count > 0 {
                let mut devices = vec![null_mut(); count as usize];
                if query(count, devices.as_mut_ptr(), &mut count) != 0 {
                    for &device in devices.iter().take(count as usize) {
                        match Self::create(egl, EGL_PLATFORM_DEVICE_EXT, device) {
                            Ok(ctx) => return Ok(ctx),
                            Err(e) => last_error = e,
                        }
                    }
                }
            }
        }
        Err(last_error)
    }
}

impl Drop for Context<'_> {
    fn drop(&mut self) {
        unsafe {
            if !self.context.is_null() {
                if (self.egl.eglGetCurrentContext)() == self.context {
                    (self.egl.eglMakeCurrent)(self.display, null_mut(), null_mut(), null_mut());
                }
                (self.egl.eglDestroyContext)(self.display, self.context);
            }
            if !self.surface.is_null() {
                (self.egl.eglDestroySurface)(self.display, self.surface);
            }
            (self.egl.eglTerminate)(self.display);
        }
    }
}

pub(crate) fn run<F>(conf: Conf, f: F)
where
    F: 'static + FnOnce() -> Box<dyn EventHandler>,
{
    let egl = LibEgl::try_load().expect("could not load EGL for offscreen rendering");
    let _api =
        unsafe { ClientApi::bind_gl(&egl) }.unwrap_or_else(|e| panic!("offscreen OpenGL: {}", e));
    let _context =
        unsafe { Context::open(&egl) }.unwrap_or_else(|e| panic!("offscreen OpenGL: {}", e));
    // Before EGL 1.5, eglGetProcAddress is allowed to omit core GL functions.
    let gl_library = Module::load("libOpenGL.so.0")
        .or_else(|_| Module::load("libGL.so.1"))
        .ok();
    gl::load_gl_funcs(|name| {
        let cname = CString::new(name).unwrap();
        unsafe { (egl.eglGetProcAddress)(cname.as_ptr()) }
            .or_else(|| gl_library.as_ref()?.get_symbol::<GlProc>(name).ok())
    });
    super::offscreen::run(&conf, f);
}
