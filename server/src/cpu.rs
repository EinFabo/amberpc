//! The `cpu` and `vulkan` builds expect AVX2, `cpu-compat` AVX; a CPU without it would die on
//! an illegal instruction. Said plainly at start instead.

/// Set by the release workflow: "cpu", "cpu-compat" or "vulkan". Unset in a build from source,
/// which suits the machine it was built on.
const BUILD: Option<&str> = option_env!("AMBER_SERVER_BUILD");

pub fn cpu_check() -> Result<(), String> {
    check(BUILD, has_avx2(), has_avx())
}

fn check(build: Option<&str>, has_avx2: bool, has_avx: bool) -> Result<(), String> {
    if build.is_some() && !has_avx {
        return Err("this CPU has no AVX (many NAS and mini-PC processors), which every Amber Server \
                    build needs - it cannot run here"
            .into());
    }
    if matches!(build, Some("cpu" | "vulkan")) && !has_avx2 {
        return Err("this CPU has no AVX2, which this build needs - install the cpu-compat build: \
                    https://github.com/EinFabo/amberpc/releases/tag/server-latest (amber-server-*-cpu-compat)"
            .into());
    }
    Ok(())
}

fn has_avx() -> bool {
    #[cfg(target_arch = "x86_64")]
    return std::is_x86_feature_detected!("avx");
    #[cfg(not(target_arch = "x86_64"))]
    true
}

fn has_avx2() -> bool {
    #[cfg(target_arch = "x86_64")]
    return std::is_x86_feature_detected!("avx2");
    #[cfg(not(target_arch = "x86_64"))]
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cpu_without_avx2_is_told_which_build_to_take() {
        let err = check(Some("cpu"), false, true).unwrap_err();
        assert!(err.contains("cpu-compat"), "{err}");
        assert!(check(Some("vulkan"), false, true).is_err());
        assert!(check(Some("cpu-compat"), false, true).is_ok());
        assert!(check(Some("cpu"), true, true).is_ok());
        assert!(check(None, false, false).is_ok(), "a build from source fits its own machine");
    }

    #[test]
    fn a_cpu_without_avx_is_told_plainly() {
        // Many NAS processors (Celeron J4125, N5105) have no AVX at all.
        let err = check(Some("cpu-compat"), false, false).unwrap_err();
        assert!(err.contains("AVX") && !err.contains("install the cpu-compat"), "{err}");
    }
}
