//! A drawable-free CGL context. The shared offscreen backend supplies the framebuffer.

use super::{
    apple::frameworks::{NSOpenGLPFAOpenGLProfiles, NSOpenGLPixelFormatAttribute},
    gl,
};
use crate::{conf::Conf, EventHandler};
use std::{
    ffi::{c_void, CString},
    ptr::null_mut,
};

type ContextObj = *mut c_void;
type PixelFormat = *mut c_void;
type GlProc = unsafe extern "C" fn();
type GetCurrent = unsafe extern "C" fn() -> ContextObj;
type SetCurrent = unsafe extern "C" fn(ContextObj) -> i32;
type Release = unsafe extern "C" fn(ContextObj);

// Reuse the window backend's lazy OpenGL.framework handle. Metal-only
// applications never load the framework.
fn symbol(name: &str) -> Option<GlProc> {
    let name = CString::new(name).unwrap();
    unsafe { super::macos::get_proc_address(name.as_ptr().cast()) }
}

// F must be the declared C function pointer type of this framework symbol.
unsafe fn required<F: Copy>(name: &str) -> F {
    let function =
        symbol(name).unwrap_or_else(|| panic!("missing OpenGL.framework entry point: {}", name));
    std::mem::transmute_copy(&function)
}

fn check(status: i32, operation: &str) {
    assert_eq!(status, 0, "{} returned CGL error {}", operation, status);
}

struct Context {
    raw: ContextObj,
    previous: ContextObj,
    get_current: GetCurrent,
    set_current: SetCurrent,
    release: Release,
}

impl Context {
    unsafe fn new() -> Self {
        let choose: unsafe extern "C" fn(*const u32, *mut PixelFormat, *mut i32) -> i32 =
            required("CGLChoosePixelFormat");
        let release_format: unsafe extern "C" fn(PixelFormat) = required("CGLReleasePixelFormat");
        let create: unsafe extern "C" fn(PixelFormat, ContextObj, *mut ContextObj) -> i32 =
            required("CGLCreateContext");
        let get_current: GetCurrent = required("CGLGetCurrentContext");
        let set_current: SetCurrent = required("CGLSetCurrentContext");
        let retain: unsafe extern "C" fn(ContextObj) -> ContextObj = required("CGLRetainContext");
        let release: Release = required("CGLReleaseContext");

        // CGL and NSOpenGL share these attributes. Allow GPUs without an attached display.
        let attrs = [
            NSOpenGLPixelFormatAttribute::NSOpenGLPFAOpenGLProfile as u32,
            NSOpenGLPFAOpenGLProfiles::NSOpenGLProfileVersion3_2Core as u32,
            NSOpenGLPixelFormatAttribute::NSOpenGLPFAAllowOfflineRenderers as u32,
            0,
        ];
        let mut format = null_mut();
        let mut count = 0;
        check(
            choose(attrs.as_ptr(), &mut format, &mut count),
            "CGLChoosePixelFormat",
        );
        assert!(!format.is_null(), "CGL returned no offscreen pixel format");
        let mut raw = null_mut();
        let status = create(format, null_mut(), &mut raw);
        release_format(format);
        check(status, "CGLCreateContext");
        assert!(!raw.is_null(), "CGL returned no offscreen context");
        let previous = get_current();
        if !previous.is_null() {
            retain(previous);
        }
        let ctx = Self {
            raw,
            previous,
            get_current,
            set_current,
            release,
        };
        check(set_current(raw), "CGLSetCurrentContext");
        ctx
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        unsafe {
            if (self.get_current)() == self.raw {
                (self.set_current)(self.previous);
            }
            (self.release)(self.raw);
            if !self.previous.is_null() {
                (self.release)(self.previous);
            }
        }
    }
}

pub(crate) fn run<F>(conf: Conf, f: F)
where
    F: 'static + FnOnce() -> Box<dyn EventHandler>,
{
    let _context = unsafe { Context::new() };
    gl::load_gl_funcs(symbol);
    super::offscreen::run(&conf, f);
}
