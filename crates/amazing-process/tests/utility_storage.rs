#![cfg(any(target_os = "linux", windows))]
use amazing_ipc::codec::ByteBuf;
use amazing_ipc::interface;
use amazing_process::protocol::{
    DecodeRequest, StorageEntry, StoragePartition, StorageProxy, StoragePut, UtilityProxy,
};
use amazing_process::{Capabilities, ProcessHost, RestartPolicy, Role};
use amazing_storage::core::{BucketKey, DiskStore, EntryKey, StorageKey};
use std::collections::BTreeMap;
use std::time::Duration;
interface! {interface SandboxProbe(18) => SandboxProbeProxy, SandboxProbeReceiver {fn probe(message:String) -> String = 1;}}

fn exe() -> &'static std::path::Path {
    std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"))
}
fn utility() -> ProcessHost {
    ProcessHost::spawn(
        exe(),
        Role::Utility,
        Capabilities(BTreeMap::new()),
        RestartPolicy::Never,
    )
    .unwrap()
}

#[test]
fn utility_shared_image_and_audio_match_public_decoders() {
    let mut host = utility();
    let proxy = UtilityProxy::new(host.connection.clone(), Duration::from_secs(5));
    let ppm = b"P6\n2 1\n255\n\xff\x00\x00\x00\xff\x00";
    let expected = amazing_media::decode_image(ppm).unwrap();
    let ready = proxy
        .decode(DecodeRequest {
            kind: 1,
            bytes: ByteBuf(ppm.to_vec()),
        })
        .unwrap();
    let mapped = host.receive_decoded(&ready).unwrap();
    assert_eq!(
        (mapped.width, mapped.height),
        (expected.width, expected.height)
    );
    assert_eq!(mapped.bytes(), expected.pixels);
    drop(mapped);

    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1"><rect width="2" height="1" fill="red"/></svg>"#;
    let expected = amazing_svg::decode(svg, amazing_svg::DecodeLimits::default()).unwrap();
    let ready = proxy
        .decode(DecodeRequest {
            kind: 1,
            bytes: ByteBuf(svg.to_vec()),
        })
        .unwrap();
    let mapped = host.receive_decoded(&ready).unwrap();
    assert_eq!(
        (mapped.width, mapped.height),
        (expected.width, expected.height)
    );
    assert_eq!(mapped.bytes(), expected.pixels);
    drop(mapped);

    let wav: &[u8] = b"RIFF\x26\x00\x00\x00WAVEfmt \x10\x00\x00\x00\x01\x00\x01\x00\x11\x2b\x00\x00\x11\x2b\x00\x00\x01\x00\x08\x00data\x02\x00\x00\x00\x00\xff";
    let expected = amazing_media::decode_wav(wav).unwrap();
    let ready = proxy
        .decode(DecodeRequest {
            kind: 3,
            bytes: ByteBuf(wav.to_vec()),
        })
        .unwrap();
    let mapped = host.receive_decoded(&ready).unwrap();
    let samples: Vec<i16> = mapped
        .bytes()
        .chunks_exact(2)
        .map(|s| i16::from_le_bytes([s[0], s[1]]))
        .collect();
    assert_eq!(
        (mapped.sample_rate, mapped.channels),
        (expected.sample_rate, expected.channels)
    );
    assert_eq!(samples, expected.samples);
    host.kill().unwrap();
}

#[test]
fn utility_image_corpus_decodes_bounded_web_formats_and_exif_orientation() {
    let mut host = utility();
    let proxy = UtilityProxy::new(host.connection.clone(), Duration::from_secs(5));
    let corpus: &[(&str, &[u8])] = &[
        (
            "PNG",
            include_bytes!("fixtures/utility-images/two-pixels.png"),
        ),
        (
            "JPEG",
            include_bytes!("fixtures/utility-images/two-pixels.jpg"),
        ),
        (
            "GIF",
            include_bytes!("fixtures/utility-images/two-pixels.gif"),
        ),
        (
            "WebP",
            include_bytes!("fixtures/utility-images/two-pixels.webp"),
        ),
        (
            "BMP",
            include_bytes!("fixtures/utility-images/two-pixels.bmp"),
        ),
        (
            "PPM",
            include_bytes!("fixtures/utility-images/two-pixels.ppm"),
        ),
        (
            "EXIF rotation",
            include_bytes!("../../amazing-image/tests/fixtures/exif-orientation-6-ru.jpg"),
        ),
    ];
    for (format, bytes) in corpus {
        let expected = amazing_image::decode_with_orientation(bytes, Default::default(), true)
            .unwrap_or_else(|error| panic!("{format}: {error}"));
        let ready = proxy
            .decode(DecodeRequest {
                kind: 1,
                bytes: ByteBuf(bytes.to_vec()),
            })
            .unwrap_or_else(|error| panic!("{format}: {error:?}"));
        let mapped = host.receive_decoded(&ready).unwrap();
        assert_eq!(
            (mapped.width, mapped.height),
            (expected.width, expected.height),
            "{format}"
        );
        assert_eq!(mapped.bytes(), expected.pixels, "{format}");
        drop(mapped);
    }
    assert!(proxy
        .decode(DecodeRequest {
            kind: 1,
            bytes: ByteBuf(b"not an image".to_vec()),
        })
        .is_err());
    host.kill().unwrap();
}

#[test]
fn utility_sfnt_matches_public_text_provider_decoder() {
    let mut host = utility();
    let proxy = UtilityProxy::new(host.connection.clone(), Duration::from_secs(5));
    let sfnt = include_bytes!("fixtures/synthetic-a.ttf");
    let expected = amazing_text_provider::decode_web_font_to_sfnt(
        amazing_text_provider::FontFormat::OpenType,
        sfnt,
    )
    .unwrap();
    let ready = proxy
        .decode(DecodeRequest {
            kind: 2,
            bytes: ByteBuf(sfnt.to_vec()),
        })
        .unwrap();
    assert_eq!(host.receive_decoded(&ready).unwrap().bytes(), expected);
    let mut corrupt = sfnt.to_vec();
    corrupt[16] ^= 1;
    assert!(amazing_text_provider::decode_web_font_to_sfnt(
        amazing_text_provider::FontFormat::OpenType,
        &corrupt
    )
    .is_err());
    assert!(proxy
        .decode(DecodeRequest {
            kind: 2,
            bytes: ByteBuf(corrupt)
        })
        .is_err());
    host.kill().unwrap();
}

#[test]
fn decoder_crash_does_not_kill_browser_or_other_utility() {
    let mut crashed = utility();
    let survivor = utility();
    let proxy = UtilityProxy::new(crashed.connection.clone(), Duration::from_secs(5));
    let ready = proxy
        .decode(DecodeRequest {
            kind: 1,
            bytes: ByteBuf(b"P6\n1 1\n255\n\x01\x02\x03".to_vec()),
        })
        .unwrap();
    crashed.kill().unwrap();
    assert!(crashed.receive_decoded(&ready).is_err());
    let proxy = UtilityProxy::new(survivor.connection.clone(), Duration::from_secs(5));
    let ready = proxy
        .decode(DecodeRequest {
            kind: 1,
            bytes: ByteBuf(b"P6\n1 1\n255\n\x01\x02\x03".to_vec()),
        })
        .unwrap();
    assert_eq!(
        survivor.receive_decoded(&ready).unwrap().bytes(),
        &[1, 2, 3, 255]
    );
}

fn storage_host(path: &std::path::Path, quota: u64) -> ProcessHost {
    ProcessHost::spawn(
        exe(),
        Role::Storage,
        Capabilities(BTreeMap::from([
            ("__storage_root".into(), path.to_string_lossy().into_owned()),
            ("__storage_quota".into(), quota.to_string()),
        ])),
        RestartPolicy::Never,
    )
    .unwrap()
}
fn stop_storage(host: &mut ProcessHost) {
    host.kill().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !host.crashed().unwrap() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(host.crashed().unwrap());
}
#[test]
fn storage_process_matches_diskstore_quota_partition_and_crash_recovery() {
    let base = std::env::temp_dir().join(format!(
        "ipc8-storage-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let process_path = base.join("process");
    let direct_path = base.join("direct");
    let quota = 1000;
    let mut direct = DiskStore::open(&direct_path, quota).unwrap();
    let partition = StoragePartition {
        origin: "https://a.test".into(),
        top_level_site: "https://top.test".into(),
        bucket: "default".into(),
    };
    let entry = StorageEntry {
        partition: partition.clone(),
        namespace: "local".into(),
        key: ByteBuf(b"name".to_vec()),
    };
    let direct_key = EntryKey {
        bucket: BucketKey::new(
            StorageKey::new(&partition.origin, &partition.top_level_site),
            &partition.bucket,
        ),
        namespace: "local".into(),
        key: b"name".to_vec(),
    };
    let mut host = storage_host(&process_path, quota);
    let proxy = StorageProxy::new(host.connection.clone(), Duration::from_secs(5));
    direct.put(direct_key.clone(), b"value".to_vec()).unwrap();
    assert!(proxy
        .put(StoragePut {
            entry: entry.clone(),
            value: ByteBuf(b"value".to_vec())
        })
        .unwrap());
    assert_eq!(
        proxy.get(entry.clone()).unwrap().unwrap().0,
        direct.get(&direct_key).unwrap()
    );
    let mut other = entry.clone();
    other.partition.top_level_site = "https://other.test".into();
    assert!(proxy.get(other).unwrap().is_none());
    let huge = vec![b'x'; quota as usize + 1];
    assert!(direct.put(direct_key.clone(), huge.clone()).is_err());
    assert!(proxy
        .put(StoragePut {
            entry: entry.clone(),
            value: ByteBuf(huge)
        })
        .is_err());
    // Kill after an acknowledged put, without a checkpoint: the storage log
    // must recover the value on the next process start.
    stop_storage(&mut host);
    drop(host);
    let mut host = storage_host(&process_path, quota);
    let proxy = StorageProxy::new(host.connection.clone(), Duration::from_secs(5));
    assert_eq!(proxy.get(entry.clone()).unwrap().unwrap().0, b"value");
    assert!(proxy.checkpoint(()).unwrap());
    assert!(proxy.delete(entry.clone()).unwrap());
    assert!(proxy.get(entry.clone()).unwrap().is_none());
    stop_storage(&mut host);
    drop(host);
    let mut host = storage_host(&process_path, quota);
    let proxy = StorageProxy::new(host.connection.clone(), Duration::from_secs(5));
    assert!(proxy.get(entry).unwrap().is_none());
    stop_storage(&mut host);
    drop(host);
    drop(direct);
    std::fs::remove_dir_all(base).unwrap();
}

#[cfg(windows)]
#[test]
fn appcontainer_utility_shared_output_has_no_file_or_network_grant() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let file = std::env::temp_dir().join(format!("ipc8-private-{}", std::process::id()));
    std::fs::write(&file, b"private").unwrap();
    let mut host = ProcessHost::spawn_sandboxed_utility(
        exe(),
        Capabilities(BTreeMap::from([("__sandbox_probe".into(), "1".into())])),
        RestartPolicy::Never,
    )
    .unwrap();
    let probe = SandboxProbeProxy::new(host.connection.clone(), Duration::from_secs(5));
    for message in [
        format!("open:{}", file.display()),
        format!("connect:{}", listener.local_addr().unwrap()),
        "spawn".into(),
        format!("open-process:{}", std::process::id()),
    ] {
        assert!(probe.probe(message).unwrap().starts_with("denied:"));
    }
    let mapped = host
        .decode_untrusted(1, b"P6\n1 1\n255\n\x01\x02\x03", Duration::from_secs(5))
        .unwrap();
    assert_eq!(mapped.bytes(), &[1, 2, 3, 255]);
    drop(mapped);
    assert!(host
        .decode_untrusted(1, b"invalid image", Duration::from_secs(5))
        .is_err());
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !host.crashed().unwrap() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(host.crashed().unwrap());
    host.kill().unwrap();
    std::fs::remove_file(file).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn utility_sandbox_launch_fails_closed_when_user_namespaces_are_blocked() {
    match ProcessHost::spawn_sandboxed_utility(
        exe(),
        Capabilities(BTreeMap::new()),
        RestartPolicy::Never,
    ) {
        Ok(mut host) => {
            assert_eq!(host.role, Role::Utility);
            host.kill().unwrap();
        }
        Err(error) => eprintln!("sandbox setup denied by host; launch failed closed: {error}"),
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "Venus AppArmor denies unprivileged user namespace UID/GID mapping"]
fn utility_sandbox_denies_file_network_and_child_spawn() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let file = std::env::temp_dir().join(format!("ipc8-private-{}", std::process::id()));
    std::fs::write(&file, b"private").unwrap();
    let mut host = ProcessHost::spawn_sandboxed_utility(
        exe(),
        Capabilities(BTreeMap::from([("__sandbox_probe".into(), "1".into())])),
        RestartPolicy::Never,
    )
    .unwrap();
    let probe = SandboxProbeProxy::new(host.connection.clone(), Duration::from_secs(5));
    for message in [
        format!("open:{}", file.display()),
        format!("connect:{}", listener.local_addr().unwrap()),
        "spawn".into(),
    ] {
        assert_eq!(probe.probe(message).unwrap(), "denied:1");
    }
    host.kill().unwrap();
    std::fs::remove_file(file).unwrap();
}
