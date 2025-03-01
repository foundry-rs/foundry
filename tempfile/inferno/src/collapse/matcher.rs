// Detects vmlinux in stack, with version or without.
//
// Examples:
//
// ffffffffb94000e0 __softirqentry_text_start+0xe0 (/usr/lib/debug/boot/vmlinux-5.4.14-cloudflare-2020.1.11)
// 8c3453 tcp_sendmsg (/lib/modules/4.3.0-rc1-virtual/build/vmlinux)
// 7d8 ipv4_conntrack_local+0x7f8f80b8 ([nf_conntrack_ipv4])
//
#[inline]
pub(super) fn is_vmlinux(s: &str) -> bool {
    if let Some(vm) = s.rfind("vmlinux") {
        s[vm..]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
    } else {
        false
    }
}

// Detect kernel from module name, module file or from vmlinux
#[inline]
pub(super) fn is_kernel(s: &str) -> bool {
    (s.starts_with('[') || s.ends_with(".ko") || is_vmlinux(s)) && s != "[unknown]"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_vmlinux_true() {
        assert!(is_vmlinux("vmlinux"));
        assert!(is_vmlinux("vmlinux-5"));
        assert!(is_vmlinux("vmlinux-54"));
        assert!(is_vmlinux("vmlinux_54"));
        assert!(is_vmlinux("vmlinux-vmlinux"));
        assert!(is_vmlinux("vmlinux-5.4.14"));
        assert!(is_vmlinux("vmlinux-54-2020"));
        assert!(is_vmlinux("vmlinux-cloudflare"));
        assert!(is_vmlinux("vmlinux_cloudflare"));
        assert!(is_vmlinux("vmlinux-cloudflare-2020.1.11"));
        assert!(is_vmlinux("vmlinux-5.4.14-cloudflare-2020.1.11"));
        assert!(is_vmlinux("/usr/lib/debug/boot/vmlinux"));
        assert!(is_vmlinux(
            "/usr/lib/debug/boot/vmlinux-5.4.14-cloudflare-2020.1.11"
        ));
    }

    #[test]
    fn is_vmlinux_false() {
        assert!(!is_vmlinux("vmlinux/"));
        assert!(!is_vmlinux("vmlinux "));
        assert!(!is_vmlinux("vmlinux+5"));
        assert!(!is_vmlinux("vmlinux,54"));
        assert!(!is_vmlinux("vmlinux\\5.4.14"));
        assert!(!is_vmlinux("vmlinux-Тест"));
        assert!(!is_vmlinux("vmlinux-cloudflare "));
        assert!(!is_vmlinux("vmlinux-5.4.14-cloudflare-2020.1.11)"));
        assert!(!is_vmlinux("/usr/lib/debug/boot/vmlinu"));
        assert!(!is_vmlinux(
            "/usr/lib/debug/boot/vmlinu-5.4.14-cloudflare-2020.1.11"
        ));
    }

    #[test]
    fn is_kernel_true() {
        assert!(is_kernel("["));
        assert!(is_kernel("[vmlinux"));
        assert!(is_kernel("[test"));
        assert!(is_kernel("[test]"));
        assert!(is_kernel(".ko"));
        assert!(is_kernel("module.ko"));
        assert!(is_kernel("vmlinux.ko"));
        assert!(is_kernel("vmlinux"));
        assert!(is_kernel(" [vmlinux"));
        assert!(is_kernel("vmlinux-5.4.14-cloudflare-2020.1.11"));
        assert!(is_kernel("vmlinux-5.4.14-cloudflare-2020.1.11"));
        assert!(is_kernel(
            "/usr/lib/debug/boot/vmlinux-5.4.14-cloudflare-2020.1.11"
        ));
    }

    #[test]
    fn is_kernel_false() {
        assert!(!is_kernel("[unknown]"));
        assert!(!is_kernel(" ["));
        assert!(!is_kernel(".ko "));
        assert!(!is_kernel(" [.ko "));
        assert!(!is_kernel("vmlinux-cloudflare "));
        assert!(!is_kernel("vmlinux-5.4.14-cloudflare-2020.1.11)"));
        assert!(!is_kernel("/usr/lib/debug/boot/vmlinu"));
        assert!(!is_kernel(
            "/usr/lib/debug/boot/vmlinu-5.4.14-cloudflare-2020.1.11"
        ));
    }
}
