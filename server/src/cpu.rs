//! The `cpu` and `vulkan` builds expect AVX2; a CPU without it would die on an illegal
//! instruction. Said plainly at start instead.

/// Set by the release workflow: "cpu", "cpu-compat" or "vulkan". Unset in a build from source,
/// which suits the machine it was built on.
const BUILD: Option<&str> = option_env!("AMBER_SERVER_BUILD");

pub fn cpu_check() -> Result<(), String> {
    check(BUILD, has_avx2())
}

fn check(build: Option<&str>, has_avx2: bool) -> Result<(), String> {
    if matches!(build, Some("cpu" | "vulkan")) && !has_avx2 {
        return Err("this CPU has no AVX2, which this build needs - install the cpu-compat build: \
                    https://github.com/EinFabo/amberpc/releases (amber-server-*-cpu-compat)"
            .into());
    }
    Ok(())
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
        let err = check(Some("cpu"), false).unwrap_err();
        assert!(err.contains("cpu-compat"), "{err}");
        assert!(check(Some("vulkan"), false).is_err());
        assert!(check(Some("cpu-compat"), false).is_ok());
        assert!(check(Some("cpu"), true).is_ok());
        assert!(check(None, false).is_ok(), "a build from source fits its own machine");
    }
}
