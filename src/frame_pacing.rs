use std::sync::atomic::{AtomicU16, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FramePacing {
    Target(u16),
    Unlimited,
}

const UNLIMITED: u16 = 0;
static REQUESTED: AtomicU16 = AtomicU16::new(60);

pub fn set_frame_pacing(mode: FramePacing) {
    let encoded = match mode {
        FramePacing::Target(fps) => fps.max(1),
        FramePacing::Unlimited => UNLIMITED,
    };
    REQUESTED.store(encoded, Ordering::Relaxed);
}

pub(crate) fn requested() -> FramePacing {
    match REQUESTED.load(Ordering::Relaxed) {
        UNLIMITED => FramePacing::Unlimited,
        fps => FramePacing::Target(fps),
    }
}
