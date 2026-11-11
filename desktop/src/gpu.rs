//! Models on the graphics processor. On Windows, llama.cpp and whisper.cpp reach it through Vulkan, whose
//! loader (vulkan-1.dll) comes with the graphics driver. The app links it to load only when first used
//! (build.rs), so it starts on a PC without one, such as a virtual machine with no graphics driver.
//!
//! There, the first call into Vulkan would fail to load it and end the program. This hook answers instead:
//! every Vulkan function says there's no driver, which ggml takes as no graphics processor, and models run
//! on the processor as before.

#[cfg(windows)]
mod missing_loader {
    use std::ffi::{c_char, c_void, CStr};

    /// From delayimp.h.
    #[repr(C)]
    #[allow(dead_code)]
    struct DelayLoadProc {
        import_by_name: i32,
        name: *const c_char,
    }

    #[repr(C)]
    #[allow(dead_code)]
    pub struct DelayLoadInfo {
        cb: u32,
        descriptor: *const c_void,
        slot: *mut *const c_void,
        dll: *const c_char,
        proc_: DelayLoadProc,
        module: *mut c_void,
        function: *const c_void,
        last_error: u32,
    }

    const FAIL_LOAD_LIB: u32 = 3;
    const FAIL_GET_PROC: u32 = 4;
    /// VK_ERROR_INCOMPATIBLE_DRIVER.
    const NO_DRIVER: i32 = -9;

    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    }

    /// Every Vulkan function that returns a result: there's no driver.
    extern "system" fn no_driver() -> i32 {
        NO_DRIVER
    }

    /// vkGetInstanceProcAddr: every function it's asked for is the one above.
    extern "system" fn get_proc(_instance: *mut c_void, _name: *const c_char) -> *const c_void {
        no_driver as *const c_void
    }

    unsafe extern "system" fn hook(notify: u32, info: *mut DelayLoadInfo) -> *const c_void {
        let Some(info) = info.as_ref() else { return std::ptr::null() };
        if info.dll.is_null() || !CStr::from_ptr(info.dll).to_bytes().eq_ignore_ascii_case(b"vulkan-1.dll") {
            return std::ptr::null();
        }
        match notify {
            // Any module will do: looking up the function in it fails, which comes back here.
            FAIL_LOAD_LIB => GetModuleHandleW(std::ptr::null()) as *const c_void,
            FAIL_GET_PROC => {
                let by_name = info.proc_.import_by_name != 0 && !info.proc_.name.is_null();
                if by_name && CStr::from_ptr(info.proc_.name).to_bytes() == b"vkGetInstanceProcAddr" {
                    get_proc as *const c_void
                } else {
                    no_driver as *const c_void
                }
            }
            _ => std::ptr::null(),
        }
    }

    pub type Hook = unsafe extern "system" fn(u32, *mut DelayLoadInfo) -> *const c_void;

    /// The delay-load helper (delayimp.lib) calls this when a library or function can't be loaded.
    #[no_mangle]
    #[used]
    pub static __pfnDliFailureHook2: Hook = hook;
}

/// Makes sure the hook is linked into the program before anything touches Vulkan.
pub fn prepare() {
    #[cfg(windows)]
    std::hint::black_box(&missing_loader::__pfnDliFailureHook2);
}
