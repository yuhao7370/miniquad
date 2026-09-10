//! Checks the opt-in contract without opening a window or requiring Android.
fn main() {
    use miniquad::conf::{AndroidGlesVersion, Platform};
    assert_eq!(Platform::default().android_gles_version as u32, 2);
    let platform = Platform {
        android_gles_version: AndroidGlesVersion::GLES3,
        ..Default::default()
    };
    assert_eq!(platform.android_gles_version as u32, 3);
}
