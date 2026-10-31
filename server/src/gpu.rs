//! Whether the Vulkan build sees a GPU it can use.

use std::ffi::CStr;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Cpu,
    Gpu,
    Igpu,
    Other,
}

/// A device worth running on: a GPU, built in or not - not Mesa's software renderer, which
/// Vulkan lists as a GPU on machines without a driver.
pub fn usable(kind: Kind, description: &str) -> bool {
    matches!(kind, Kind::Gpu | Kind::Igpu) && !description.to_lowercase().contains("llvmpipe")
}

/// The usable GPUs ggml's Vulkan backend finds (none in a build without it). Asked once:
/// looking starts Vulkan.
pub fn vulkan_devices() -> &'static [String] {
    static DEVICES: OnceLock<Vec<String>> = OnceLock::new();
    DEVICES.get_or_init(|| {
        use whisper_rs::whisper_rs_sys as sys;
        let mut found = Vec::new();
        // SAFETY: ggml's registry calls; the strings are ggml's own, copied at once.
        unsafe {
            let reg = sys::ggml_backend_reg_by_name(c"Vulkan".as_ptr());
            if reg.is_null() {
                return found;
            }
            for i in 0..sys::ggml_backend_reg_dev_count(reg) {
                let dev = sys::ggml_backend_reg_dev_get(reg, i);
                if dev.is_null() {
                    continue;
                }
                let kind = match sys::ggml_backend_dev_type(dev) {
                    sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_CPU => Kind::Cpu,
                    sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU => Kind::Gpu,
                    sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_IGPU => Kind::Igpu,
                    _ => Kind::Other,
                };
                let text = sys::ggml_backend_dev_description(dev);
                let description = if text.is_null() { String::new() } else { CStr::from_ptr(text).to_string_lossy().into_owned() };
                if usable(kind, &description) {
                    found.push(description);
                }
            }
        }
        found
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_real_gpus_count() {
        assert!(usable(Kind::Gpu, "NVIDIA GeForce RTX 3060"));
        assert!(usable(Kind::Igpu, "AMD Radeon Graphics"));
        assert!(!usable(Kind::Gpu, "llvmpipe (LLVM 15.0.7, 256 bits)"), "Mesa's software renderer");
        assert!(!usable(Kind::Cpu, "CPU"));
    }

    #[test]
    fn a_build_without_vulkan_sees_none() {
        if !cfg!(feature = "vulkan") {
            assert!(vulkan_devices().is_empty());
        }
    }
}
