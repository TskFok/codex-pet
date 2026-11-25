use pet_core::{SessionKey, Source};
use pet_desktop::navigation::{
    Desktop, NavigationError, OpenIntent, OpenOutcome, Registration, WebTarget, open_session,
    open_web_session,
};
use std::path::PathBuf;

const INSTALL: &str = "a4cbcea7-78fa-45fd-93e9-2469acd56f3c";
const THREAD: &str = "019887a2-718b-78ab-a612-6a0361ac316d";

fn key(source: Source, id: &str) -> SessionKey {
    SessionKey {
        source,
        installation_id: INSTALL.into(),
        session_id: id.into(),
    }
}

fn target(source: Source, supports_session: bool) -> Registration {
    Registration {
        source,
        installation_id: INSTALL.into(),
        desktop: Some(Desktop {
            path: std::env::temp_dir().join("本地应用 & 测试"),
            supports_session,
        }),
    }
}

#[test]
fn official_codex_thread_is_one_argument_to_the_registered_host() {
    let target = target(Source::Codex, true);
    let mut requests = Vec::new();
    let result = open_session(
        &key(Source::Codex, THREAD),
        Some(&target),
        OpenIntent::Session,
        |path, url| {
            requests.push((path.to_owned(), url.map(str::to_owned)));
            Ok(())
        },
    );
    assert_eq!(result, Ok(OpenOutcome::Requested));
    assert_eq!(
        requests,
        vec![(
            target.desktop.unwrap().path,
            Some(format!("codex://threads/{THREAD}")),
        )]
    );
}

#[test]
fn harness_encodes_one_session_path_segment() {
    let result = open_session(
        &key(Source::Deepseek, "会话/a ?#%&"),
        Some(&target(Source::Deepseek, true)),
        OpenIntent::Session,
        |_, url| {
            assert_eq!(
                url,
                Some("dsh://session/%E4%BC%9A%E8%AF%9D%2Fa%20%3F%23%25%26")
            );
            Ok(())
        },
    );
    assert_eq!(result, Ok(OpenOutcome::Requested));
}

#[test]
fn invalid_or_unregistered_input_never_dispatches() {
    let codex = target(Source::Codex, true);
    let harness = target(Source::Deepseek, true);
    let mut unknown = key(Source::Codex, THREAD);
    unknown.installation_id = "12d59cdc-1904-43b3-acbf-44114fd82332".into();
    for (key, registration, expected) in [
        (unknown, &codex, NavigationError::UnknownInstallation),
        (
            key(Source::Deepseek, "test"),
            &codex,
            NavigationError::UnknownInstallation,
        ),
        (
            key(Source::Codex, "https://evil.example"),
            &codex,
            NavigationError::InvalidSessionId,
        ),
        (
            key(Source::Deepseek, "https://evil.example"),
            &harness,
            NavigationError::InvalidSessionId,
        ),
        (
            key(Source::Deepseek, "x\u{0085}y"),
            &harness,
            NavigationError::InvalidSessionId,
        ),
        (
            key(Source::Deepseek, "x\u{0000}y"),
            &harness,
            NavigationError::InvalidSessionId,
        ),
        (
            key(Source::Deepseek, ".."),
            &harness,
            NavigationError::InvalidSessionId,
        ),
    ] {
        assert_eq!(
            open_session(
                &key,
                Some(registration),
                OpenIntent::Session,
                |_, _| panic!("无效目标不能派发")
            ),
            Err(expected)
        );
    }
    for path in [
        std::env::temp_dir().join("病\n例"),
        PathBuf::from("relative.exe"),
        PathBuf::from("/Applications/../bin/sh"),
        PathBuf::from(r"\\server\share\Codex.exe"),
    ] {
        let mut target = target(Source::Codex, true);
        target.desktop.as_mut().unwrap().path = path;
        assert_eq!(
            open_session(
                &key(Source::Codex, THREAD),
                Some(&target),
                OpenIntent::Session,
                |_, _| panic!("无效路径不能派发")
            ),
            Err(NavigationError::InvalidHostPath),
        );
    }
}

#[test]
fn incompatible_host_requires_explicit_application_request() {
    let target = target(Source::Deepseek, false);
    let key = key(Source::Deepseek, "one");
    assert_eq!(
        open_session(&key, Some(&target), OpenIntent::Session, |_, _| panic!(
            "不能隐式打开应用"
        )),
        Ok(OpenOutcome::ApplicationOnly { opened: false }),
    );
    assert_eq!(
        open_session(&key, Some(&target), OpenIntent::Application, |_, url| {
            assert_eq!(url, Some("dsh://open"));
            Ok(())
        }),
        Ok(OpenOutcome::ApplicationOnly { opened: true }),
    );
}

#[test]
fn missing_desktop_and_rejected_system_request_have_distinct_results() {
    let k = key(Source::Codex, THREAD);
    let missing = Registration {
        source: Source::Codex,
        installation_id: INSTALL.into(),
        desktop: None,
    };
    assert_eq!(
        open_session(&k, Some(&missing), OpenIntent::Session, |_, _| panic!(
            "未安装宿主不能派发"
        )),
        Ok(OpenOutcome::Unavailable),
    );
    assert_eq!(
        open_session(
            &k,
            Some(&target(Source::Codex, true)),
            OpenIntent::Session,
            |_, _| Err(std::io::Error::other("系统拒绝"))
        ),
        Err(NavigationError::DispatchFailed),
    );
}

#[test]
fn harness_web_session_dispatches_encoded_loopback_hash_and_gates_application_only() {
    let session = key(Source::Deepseek, "会话/a ?#%&");
    let supported = WebTarget {
        origin: "http://127.0.0.1:3080".into(),
        supports_session: true,
    };
    assert_eq!(
        open_web_session(
            &session,
            Source::Deepseek,
            INSTALL,
            Some(&supported),
            OpenIntent::Session,
            |url| {
                assert_eq!(
                    url,
                    "http://127.0.0.1:3080/#codex-pet-session=%E4%BC%9A%E8%AF%9D%2Fa%20%3F%23%25%26"
                );
                Ok(())
            },
        ),
        Ok(OpenOutcome::Requested),
    );

    let legacy_client = WebTarget {
        origin: "http://[::1]:3080/".into(),
        supports_session: false,
    };
    assert_eq!(
        open_web_session(
            &session,
            Source::Deepseek,
            INSTALL,
            Some(&legacy_client),
            OpenIntent::Session,
            |_| panic!("旧版 Web 客户端不能隐式打开应用"),
        ),
        Ok(OpenOutcome::ApplicationOnly { opened: false }),
    );
    assert_eq!(
        open_web_session(
            &session,
            Source::Deepseek,
            INSTALL,
            Some(&legacy_client),
            OpenIntent::Application,
            |url| {
                assert_eq!(url, "http://[::1]:3080/");
                Ok(())
            },
        ),
        Ok(OpenOutcome::ApplicationOnly { opened: true }),
    );

    assert_eq!(
        open_web_session(
            &session,
            Source::Deepseek,
            INSTALL,
            None,
            OpenIntent::Session,
            |_| panic!("未运行本地 Web 服务不能派发"),
        ),
        Ok(OpenOutcome::Unavailable),
    );
}

#[test]
fn harness_web_session_rejects_non_loopback_origins_and_invalid_inputs() {
    let session = key(Source::Deepseek, "session-1");
    for origin in [
        "https://evil.example:3080",
        "http://0.0.0.0:3080",
        "http://127.0.0.1:0",
        "http://127.0.0.1:3080/path",
        "http://evil@127.0.0.1:3080",
        "javascript:alert(1)",
    ] {
        let bad = WebTarget {
            origin: origin.into(),
            supports_session: true,
        };
        assert_eq!(
            open_web_session(
                &session,
                Source::Deepseek,
                INSTALL,
                Some(&bad),
                OpenIntent::Session,
                |_| panic!("非法来源不能派发: {origin}"),
            ),
            Err(NavigationError::InvalidHostPath),
            "{origin}"
        );
    }

    let ok_target = WebTarget {
        origin: "http://localhost:3080".into(),
        supports_session: true,
    };
    assert_eq!(
        open_web_session(
            &session,
            Source::Deepseek,
            "12d59cdc-1904-43b3-acbf-44114fd82332",
            Some(&ok_target),
            OpenIntent::Session,
            |_| panic!("未登记安装不能派发"),
        ),
        Err(NavigationError::UnknownInstallation),
    );
    assert_eq!(
        open_web_session(
            &key(Source::Deepseek, "https://evil.example"),
            Source::Deepseek,
            INSTALL,
            Some(&ok_target),
            OpenIntent::Session,
            |_| panic!("非法会话标识不能派发"),
        ),
        Err(NavigationError::InvalidSessionId),
    );
}
