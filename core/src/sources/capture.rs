//! Shared specification for automatic credential capture.
//!
//! Instead of sending the user into devtools to hunt for a cookie or bearer
//! token (see the README's manual instructions, kept as a fallback), the
//! desktop app opens a real embedded browser window on the source's own
//! login page and reads the resulting session back out of the webview's
//! cookie store once login completes.
//!
//! This module is deliberately free of any webview/UI dependency -- it only
//! knows *what* to look for and *how* to assemble it into the value
//! `Config` expects, so both the spec and the assembly logic are unit
//! tested here rather than duplicated (and left untested) in `desktop/`.
//! The window/webview mechanics live in `desktop/src-tauri`, the only crate
//! with a webview to embed.

/// One `(name, value)` cookie pair, decoupled from any particular webview
/// library's cookie type -- callers convert whatever cookie jar they read
/// into this before calling into this module.
pub type CookiePair = (String, String);

/// Synthetic cookie name the injected "I'm logged in" banner button sets.
/// Used as the completion signal for sources where no single well-known
/// cookie reliably indicates a finished login (Manning spreads its session
/// across several cookies with no documented, stable name to key off of;
/// Packt requires two cookies together -- `packt_session` and
/// `XSRF-TOKEN` -- so the same reasoning applies).
pub const MANUAL_READY_SIGNAL_COOKIE: &str = "__library_capture_ready";

/// How a source's capture completes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Completion {
    /// Capture completes the instant this cookie appears in the polled jar
    /// -- its value *is* the captured credential, no button click needed.
    /// Only safe when the cookie is provably absent/different pre-login.
    AutoCookie(&'static str),
    /// Capture completes once the user clicks the injected "I'm logged in"
    /// banner; the value is this one cookie's value at that point. Needed
    /// whenever the named cookie also exists for anonymous/guest visitors,
    /// so its mere presence can't prove a completed login.
    ManualCookie(&'static str),
    /// Capture completes once the user clicks the banner; the value is
    /// every real cookie for the polled domains, joined into a
    /// `name=value; name=value` jar string. For sources with no single
    /// well-known session cookie to key off of.
    ManualCookieJar,
}

/// Describes how to capture one source's credential via an embedded login
/// window.
pub struct CaptureSpec {
    /// Matches `model::Source::as_str()` for the live sources (`humble_bundle`,
    /// `packt`, `manning`).
    pub source: &'static str,
    /// Human-readable name shown in the login window's title bar.
    pub label: &'static str,
    /// Page to open in the embedded login window.
    pub login_url: &'static str,
    /// Cookie-store domains to read back after every poll.
    pub cookie_domains: &'static [&'static str],
    pub completion: Completion,
}

pub const HUMBLE_CAPTURE: CaptureSpec = CaptureSpec {
    source: "humble_bundle",
    label: "Humble Bundle",
    login_url: "https://www.humblebundle.com/login",
    cookie_domains: &["https://www.humblebundle.com"],
    // `_simpleauth_sess` is set for anonymous visitors too (it's the
    // session-cookie mechanism itself, not a "logged in" flag), so this
    // can't auto-detect -- wait for the user's explicit confirmation, then
    // read whatever the cookie holds at that point.
    completion: Completion::ManualCookie("_simpleauth_sess"),
};

pub const PACKT_CAPTURE: CaptureSpec = CaptureSpec {
    source: "packt",
    label: "Packt",
    login_url: "https://subscription.packtpub.com/login",
    cookie_domains: &[
        "https://subscription.packtpub.com",
        "https://www.packtpub.com",
    ],
    completion: Completion::ManualCookieJar,
};

pub const MANNING_CAPTURE: CaptureSpec = CaptureSpec {
    source: "manning",
    label: "Manning",
    login_url: "https://www.manning.com/dashboard",
    cookie_domains: &["https://www.manning.com", "https://login.manning.com"],
    completion: Completion::ManualCookieJar,
};

pub const CAPTURE_SPECS: &[&CaptureSpec] = &[&HUMBLE_CAPTURE, &PACKT_CAPTURE, &MANNING_CAPTURE];

/// Looks up the capture spec for a `model::Source::as_str()` id.
pub fn spec_for(source: &str) -> Option<&'static CaptureSpec> {
    CAPTURE_SPECS.iter().copied().find(|s| s.source == source)
}

fn find_cookie<'a>(cookies: &'a [CookiePair], name: &str) -> Option<&'a str> {
    cookies
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

/// True once the "I'm logged in" banner has been clicked.
fn manual_ready(cookies: &[CookiePair]) -> bool {
    find_cookie(cookies, MANUAL_READY_SIGNAL_COOKIE).is_some()
}

/// Joins every real cookie (excluding this module's internal signal
/// cookie) into the `name=value; name=value` jar string a raw `Cookie`
/// header expects -- exactly what a browser would send, and exactly what
/// `Manning::fetch_dashboard`, `Packt::fetch`, and the README's manual
/// instructions expect.
fn build_cookie_jar(cookies: &[CookiePair]) -> String {
    cookies
        .iter()
        .filter(|(name, _)| name != MANUAL_READY_SIGNAL_COOKIE)
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Given the cookies currently in the login window's jar (restricted to
/// `spec.cookie_domains`), returns the assembled credential once capture is
/// complete, or `None` if login/confirmation hasn't happened yet.
pub fn evaluate_capture(spec: &CaptureSpec, cookies: &[CookiePair]) -> Option<String> {
    match spec.completion {
        Completion::AutoCookie(name) => find_cookie(cookies, name).map(str::to_string),
        Completion::ManualCookie(name) => manual_ready(cookies)
            .then(|| find_cookie(cookies, name))
            .flatten()
            .map(str::to_string),
        Completion::ManualCookieJar => manual_ready(cookies).then(|| build_cookie_jar(cookies)),
    }
}

const BANNER_JS: &str = r#"(function () {
  var READY_COOKIE = "__library_capture_ready";
  function addBanner() {
    if (document.getElementById("library-capture-banner") || !document.body) return;
    var btn = document.createElement("button");
    btn.id = "library-capture-banner";
    btn.textContent = "Library: I'm logged in \u2014 capture now";
    btn.style.cssText =
      "position:fixed;bottom:16px;right:16px;z-index:2147483647;padding:10px 16px;" +
      "background:#1a73e8;color:#fff;border:none;border-radius:6px;font:14px sans-serif;" +
      "cursor:pointer;box-shadow:0 2px 8px rgba(0,0,0,.35);";
    btn.onclick = function () {
      document.cookie = READY_COOKIE + "=1; path=/; max-age=3600";
      btn.textContent = "Captured \u2014 you can close this window";
      btn.disabled = true;
      btn.style.opacity = "0.6";
    };
    document.body.appendChild(btn);
  }
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", addBanner);
  } else {
    addBanner();
  }
  new MutationObserver(addBanner).observe(document.documentElement, { childList: true, subtree: true });
})();"#;

/// Builds the script to inject into the login window on every
/// navigation/frame. Only sources that complete via the manual "I'm logged
/// in" banner get it. `AutoCookie` needs no page script at all -- it
/// completes on its own with no user action.
pub fn injected_script(spec: &CaptureSpec) -> String {
    match spec.completion {
        Completion::ManualCookie(_) | Completion::ManualCookieJar => BANNER_JS.to_string(),
        Completion::AutoCookie(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(name: &str, value: &str) -> CookiePair {
        (name.to_string(), value.to_string())
    }

    #[test]
    fn spec_for_looks_up_by_source_id() {
        assert_eq!(spec_for("humble_bundle").unwrap().label, "Humble Bundle");
        assert_eq!(spec_for("packt").unwrap().label, "Packt");
        assert_eq!(spec_for("manning").unwrap().label, "Manning");
        assert!(spec_for("kindle").is_none());
        assert!(spec_for("nonsense").is_none());
    }

    #[test]
    fn humble_ignores_guest_session_cookie_until_manually_confirmed() {
        let spec = &HUMBLE_CAPTURE;
        assert_eq!(evaluate_capture(spec, &[]), None);

        // Humble sets `_simpleauth_sess` for anonymous visitors too, so its
        // mere presence must NOT be enough to capture -- this guards
        // against silently saving a guest session as if it were logged in.
        let guest_cookies = [pair("_simpleauth_sess", "guest-value")];
        assert_eq!(evaluate_capture(spec, &guest_cookies), None);

        let confirmed_cookies = [
            pair("_simpleauth_sess", "authenticated-value"),
            pair(MANUAL_READY_SIGNAL_COOKIE, "1"),
        ];
        assert_eq!(
            evaluate_capture(spec, &confirmed_cookies),
            Some("authenticated-value".to_string())
        );
    }

    #[test]
    fn auto_cookie_completes_without_manual_confirmation() {
        let spec = CaptureSpec {
            source: "test",
            label: "Test",
            login_url: "https://example.com",
            cookie_domains: &["https://example.com"],
            completion: Completion::AutoCookie("session"),
        };
        assert_eq!(evaluate_capture(&spec, &[]), None);
        let cookies = [pair("session", "value")];
        assert_eq!(evaluate_capture(&spec, &cookies), Some("value".to_string()));
    }

    #[test]
    fn packt_requires_manual_ready_signal_then_joins_jar() {
        let spec = &PACKT_CAPTURE;
        let cookies_not_ready = [
            pair("packt_session", "sess123"),
            pair("XSRF-TOKEN", "tok456"),
        ];
        assert_eq!(evaluate_capture(spec, &cookies_not_ready), None);

        let cookies_ready = [
            pair("packt_session", "sess123"),
            pair("XSRF-TOKEN", "tok456"),
            pair(MANUAL_READY_SIGNAL_COOKIE, "1"),
        ];
        assert_eq!(
            evaluate_capture(spec, &cookies_ready),
            Some("packt_session=sess123; XSRF-TOKEN=tok456".to_string())
        );
    }

    #[test]
    fn manning_requires_manual_ready_signal_then_joins_jar() {
        let spec = &MANNING_CAPTURE;
        let cookies_not_ready = [pair("session_id", "abc"), pair("csrf", "def")];
        assert_eq!(evaluate_capture(spec, &cookies_not_ready), None);

        let cookies_ready = [
            pair("session_id", "abc"),
            pair("csrf", "def"),
            pair(MANUAL_READY_SIGNAL_COOKIE, "1"),
        ];
        assert_eq!(
            evaluate_capture(spec, &cookies_ready),
            Some("session_id=abc; csrf=def".to_string())
        );
    }

    #[test]
    fn build_cookie_jar_excludes_signal_cookies() {
        let cookies = [
            pair("a", "1"),
            pair(MANUAL_READY_SIGNAL_COOKIE, "1"),
            pair("b", "2"),
        ];
        assert_eq!(build_cookie_jar(&cookies), "a=1; b=2");
    }

    #[test]
    fn injected_script_is_specific_to_each_source_completion_mode() {
        // Humble, Packt and Manning all complete via the manual banner.
        let humble_script = injected_script(&HUMBLE_CAPTURE);
        assert!(humble_script.contains("library-capture-banner"));

        let packt_script = injected_script(&PACKT_CAPTURE);
        assert!(packt_script.contains("library-capture-banner"));

        let manning_script = injected_script(&MANNING_CAPTURE);
        assert!(manning_script.contains("library-capture-banner"));

        // AutoCookie sources get no injected script at all.
        let auto_spec = CaptureSpec {
            source: "test",
            label: "Test",
            login_url: "https://example.com",
            cookie_domains: &["https://example.com"],
            completion: Completion::AutoCookie("session"),
        };
        assert_eq!(injected_script(&auto_spec), "");
    }
}
