//! 夹具驱动的 adapter 集成测试（真实样例形状，禁止补造业务字段）。

use starquake_net::adapter::ingest_wolfx_eqlist;
use starquake_net::{channel, NetEvent};
use std::fs;
use std::path::PathBuf;

fn fixture(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn wolfx_cenc_fixture_parses_record() {
    let body = fixture("wolfx/cenc_eqlist_no1.json");
    let (tx, mut rx) = channel();
    ingest_wolfx_eqlist(&tx, &body);
    let NetEvent::Record(rec) = rx.try_recv().expect("record") else {
        panic!("expected Record");
    };
    assert!((rec.magnitude - 6.0).abs() < 1e-9);
    assert!((rec.depth_km - 10.0).abs() < 1e-9);
    assert!((rec.latitude - 7.5).abs() < 1e-9);
    assert!((rec.longitude - (-80.6)).abs() < 1e-9);
    assert_eq!(rec.place, "巴拿马");
}

#[test]
fn wolfx_cenc_fixture_skips_missing_depth() {
    let body = fixture("wolfx/cenc_eqlist_no_depth.json");
    let (tx, mut rx) = channel();
    ingest_wolfx_eqlist(&tx, &body);
    assert!(
        rx.try_recv().is_err(),
        "missing depth must not invent a default"
    );
}
