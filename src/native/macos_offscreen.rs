//! A drawable-free CGL context. The shared offscreen backend supplies the framebuffer.

use super::{
    apple::frameworks::{NSOpenGLPFAOpenGLProfiles, NSOpenGLPixelFormatAttribute},
    gl,
};
use crate::{conf::Conf, EventHandler};
use std::{
    ffi::{c_char, c_void, CStr, CString},
    ptr::null_mut,
};

type CglPixelFormat = *mut c_void;
type CglContext = *mut c_void;

#[link(name = "OpenGL", kind = "framework")]
extern "C" {
    fn CGLChoosePixelFormat(
        attributes: *const u32,
        format: *mut CglPixelFormat,
        count: *mut i32,
    ) -> i32;
    fn CGLReleasePixelFormat(format: CglPixelFormat);
    fn CGLCreateContext(format: CglPixelFormat, share: CglContext, context: *mut CglContext)
        -> i32;
    fn CGLGetCurrentContext() -> CglContext;
    fn CGLSetCurrentContext(context: CglContext) -> i32;
    fn CGLRetainContext(context: CglContext) -> CglContext;
    fn CGLReleaseContext(context: CglContext);
    fn CGLErrorString(error: i32) -> *const c_char;
}

extern "C" {
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

fn check(status: i32, operation: &str) {
    if status != 0 {
        let message = unsafe { CGLErrorString(status) };
        if !message.is_null() {
            panic!(
                "{operation}: {}",
                unsafe { CStr::from_ptr(message) }.to_string_lossy()
            );
        }
        panic!("{}: CGL error {}", operation, status);
    }
}

struct Context {
    raw: CglContext,
    previous: CglContext,
}

impl Context {
    unsafe fn new() -> Self {
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
            CGLChoosePixelFormat(attrs.as_ptr(), &mut format, &mut count),
            "CGLChoosePixelFormat",
        );
        assert!(!format.is_null(), "CGL returned no offscreen pixel format");
        let mut raw = null_mut();
        let status = CGLCreateContext(format, null_mut(), &mut raw);
        CGLReleasePixelFormat(format);
        check(status, "CGLCreateContext");
        assert!(!raw.is_null(), "CGL returned no offscreen context");
        let previous = CGLGetCurrentContext();
        if !previous.is_null() {
            CGLRetainContext(previous);
        }
        let ctx = Self { raw, previous };
        check(CGLSetCurrentContext(raw), "CGLSetCurrentContext");
        ctx
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        unsafe {
            if CGLGetCurrentContext() == self.raw {
                CGLSetCurrentContext(self.previous);
            }
            CGLReleaseContext(self.raw);
            if !self.previous.is_null() {
                CGLReleaseContext(self.previous);
            }
        }
    }
}

pub(crate) fn run<F>(conf: Conf, f: F)
where
    F: 'static + FnOnce() -> Box<dyn EventHandler>,
{
    let _context = unsafe { Context::new() };
    // The linked OpenGL framework exports both the CGL and the GL entry points.
    const RTLD_DEFAULT: *mut c_void = -2_isize as *mut c_void;
    gl::load_gl_funcs(|name| {
        let name = CString::new(name).unwrap();
        let symbol = unsafe { dlsym(RTLD_DEFAULT, name.as_ptr()) };
        if symbol.is_null() {
            None
        } else {
            Some(unsafe { std::mem::transmute::<*mut c_void, unsafe extern "C" fn()>(symbol) })
        }
    });
    super::offscreen::run(&conf, f);
}
