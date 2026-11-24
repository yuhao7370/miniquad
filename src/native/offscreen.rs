//! A desktop event loop whose default render target is an application-owned FBO.
use super::gl::*;
use super::{Clipboard, NativeDisplayData, Request};
use crate::{conf::Conf, EventHandler, FramePacing};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Default)]
struct LocalClipboard(Option<String>);

impl Clipboard for LocalClipboard {
    fn get(&mut self) -> Option<String> {
        self.0.clone()
    }

    fn set(&mut self, value: &str) {
        self.0 = Some(value.to_owned());
    }
}

struct Framebuffer {
    id: GLuint,
    color: GLuint,
    depth_stencil: GLuint,
    size: (i32, i32),
}

impl Framebuffer {
    fn new(width: i32, height: i32) -> Self {
        let mut fb = Self {
            id: 0,
            color: 0,
            depth_stencil: 0,
            size: (0, 0),
        };
        unsafe {
            glGenFramebuffers(1, &mut fb.id);
            glGenRenderbuffers(1, &mut fb.color);
            glGenRenderbuffers(1, &mut fb.depth_stencil);
        }
        fb.resize(width, height);
        fb
    }

    fn resize(&mut self, width: i32, height: i32) {
        // Keep the FBO name stable: GlContext reads the initial binding once
        // to implement begin_default_pass and restore the target after readback.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, self.id);
            glBindRenderbuffer(GL_RENDERBUFFER, self.color);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, width, height);
            glFramebufferRenderbuffer(
                GL_FRAMEBUFFER,
                GL_COLOR_ATTACHMENT0,
                GL_RENDERBUFFER,
                self.color,
            );
            glBindRenderbuffer(GL_RENDERBUFFER, self.depth_stencil);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8, width, height);
            for attachment in [GL_DEPTH_ATTACHMENT, GL_STENCIL_ATTACHMENT] {
                glFramebufferRenderbuffer(
                    GL_FRAMEBUFFER,
                    attachment,
                    GL_RENDERBUFFER,
                    self.depth_stencil,
                );
            }
            assert_eq!(
                glCheckFramebufferStatus(GL_FRAMEBUFFER),
                GL_FRAMEBUFFER_COMPLETE,
                "incomplete offscreen framebuffer ({}x{})",
                width,
                height
            );
            glBindRenderbuffer(GL_RENDERBUFFER, 0);
            glViewport(0, 0, width, height);
            glScissor(0, 0, width, height);
        }
        self.size = (width, height);
    }
}

impl Drop for Framebuffer {
    fn drop(&mut self) {
        unsafe {
            glDeleteFramebuffers(1, &self.id);
            glDeleteRenderbuffers(1, &self.color);
            glDeleteRenderbuffers(1, &self.depth_stencil);
        }
    }
}

pub(crate) fn run<F>(conf: &Conf, f: F)
where
    F: 'static + FnOnce() -> Box<dyn EventHandler>,
{
    let width = conf.window_width.max(1);
    let height = conf.window_height.max(1);
    let blocking = conf.platform.blocking_event_loop;
    let (tx, rx) = mpsc::channel();
    crate::set_display(NativeDisplayData {
        blocking_event_loop: blocking,
        ..NativeDisplayData::new(width, height, tx, Box::<LocalClipboard>::default())
    });
    let mut fb = Framebuffer::new(width, height);
    let mut events = f();
    let mut update = true;
    loop {
        let started = Instant::now();
        let (ordered, requested) = {
            let display = crate::native_display().lock().unwrap();
            (display.quit_ordered, display.quit_requested)
        };
        if ordered {
            break;
        }
        if requested {
            events.quit_requested_event();
            if crate::native_display().lock().unwrap().quit_requested {
                break;
            }
        }
        let first = if blocking && !update {
            rx.recv_timeout(Duration::from_millis(50)).ok()
        } else {
            None
        };
        for request in first.into_iter().chain(rx.try_iter()) {
            match request {
                Request::ScheduleUpdate => update = true,
                Request::SetWindowSize {
                    new_width,
                    new_height,
                } => {
                    let size = (
                        new_width.clamp(1, i32::MAX as u32) as i32,
                        new_height.clamp(1, i32::MAX as u32) as i32,
                    );
                    if fb.size != size {
                        fb.resize(size.0, size.1);
                        {
                            let mut display = crate::native_display().lock().unwrap();
                            (display.screen_width, display.screen_height) = size;
                        }
                        events.resize_event(size.0 as f32, size.1 as f32);
                        update = true;
                    }
                }
                _ => {}
            }
        }
        if update || !blocking {
            update = false;
            events.update();
            events.draw();
            unsafe { glFlush() };
            if let FramePacing::Target(fps) = crate::frame_pacing::requested() {
                let period = Duration::from_secs_f64(1.0 / f64::from(fps));
                std::thread::sleep(period.saturating_sub(started.elapsed()));
            }
        }
    }
}
