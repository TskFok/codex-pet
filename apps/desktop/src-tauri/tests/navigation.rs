use pet_core::{SessionKey, Source};
use pet_desktop::navigation::{
    Desktop, NavigationError, OpenIntent, OpenOutcome, Registration, open_session,
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
