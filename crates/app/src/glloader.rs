//! GL proc-address loader for the libmpv render API (GLX then EGL).

use libloading::Library;
use std::ffi::c_void;
use std::sync::OnceLock;

type ProcFn = unsafe extern "C" fn(*const u8) -> *mut c_void;

struct Loaders {
    glx: Option<(Library, ProcFn)>,
    egl: Option<(Library, ProcFn)>,
}

fn loaders() -> &'static Loaders {
    static LOADERS: OnceLock<Loaders> = OnceLock::new();
    LOADERS.get_or_init(|| {
        let glx = unsafe {
            Library::new("libGL.so.1").ok().and_then(|l| {
                let f = l.get::<ProcFn>(b"glXGetProcAddressARB").ok().map(|f| *f);
                f.map(|f| (l, f))
            })
        };
        let egl = unsafe {
            Library::new("libEGL.so.1").ok().and_then(|l| {
                let f = l.get::<ProcFn>(b"eglGetProcAddress").ok().map(|f| *f);
                f.map(|f| (l, f))
            })
        };
        Loaders { glx, egl }
    })
}

/// Resolve a GL function pointer by name (GLX first, EGL fallback).
/// Returns null when nothing resolves; mpv treats that as "unknown".
pub fn get_proc_address(name: &str) -> *mut c_void {
    let l = loaders();
    // C strings need a NUL terminator; interior NULs cannot resolve anyway.
    let Ok(name) = std::ffi::CString::new(name) else {
        return std::ptr::null_mut();
    };
    unsafe {
        if let Some((_, glx)) = l.glx {
            let p = glx(name.as_ptr().cast());
            if !p.is_null() {
                return p;
            }
        }
        if let Some((_, egl)) = l.egl {
            return egl(name.as_ptr().cast());
        }
    }
    std::ptr::null_mut()
}
