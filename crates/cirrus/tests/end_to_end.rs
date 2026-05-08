//! End-to-end acceptance tests for cirrus.

use std::sync::Arc;

use cirrus::backends::soft::{SoftDetector, SoftMotor};
use cirrus::callbacks::CapturingSink;
use cirrus::prelude::*;

#[tokio::test]
async fn count_plan_emits_expected_document_sequence() {
    // 1 detector, 5 iterations  →  Start, Descriptor, 5 × Event, Stop  =  8 docs.
    let det = SoftDetector::new("det1");
    let sink = Arc::new(CapturingSink::new());
    let re = RunEngine::new(vec![sink.clone() as Arc<dyn DocumentSink>]);

    let plan = cirrus::ophyd_async::count(vec![det.clone()], 5);
    let result = re.run_async(plan).await.expect("plan failed");
    assert_eq!(result.exit_status, "success");

    let docs = sink.snapshot().await;
    assert_eq!(docs.len(), 8, "expected 8 documents, got {}", docs.len());

    use cirrus_core::Document::*;
    assert!(matches!(&docs[0], Start(_)), "doc 0 is RunStart");
    assert!(matches!(&docs[1], Descriptor(_)), "doc 1 is Descriptor");
    for (i, d) in docs.iter().enumerate().take(7).skip(2) {
        assert!(matches!(d, Event(_)), "doc {i} is Event");
    }
    assert!(matches!(&docs[7], Stop(_)), "last doc is RunStop");

    // RunStart and RunStop should reference each other.
    if let (Start(start), Stop(stop)) = (&docs[0], &docs[7]) {
        assert_eq!(stop.run_start, start.uid);
        assert_eq!(stop.exit_status, "success");
        assert_eq!(stop.num_events.get("primary").copied(), Some(5));
    }
}

#[tokio::test]
async fn scan_plan_emits_motor_and_detector_readings() {
    let det = SoftDetector::new("det1");
    let motor = Arc::new(SoftMotor::new("m1", Some(0.0)));
    let sink = Arc::new(CapturingSink::new());
    let re = RunEngine::new(vec![sink.clone() as Arc<dyn DocumentSink>]);

    let plan = cirrus::ophyd_async::scan(
        vec![det.clone() as Arc<dyn cirrus_core::msg::ReadableObj>],
        motor.clone() as Arc<dyn cirrus_core::msg::MovableObj>,
        motor.clone() as Arc<dyn cirrus_core::msg::ReadableObj>,
        0.0,
        4.0,
        5,
    );
    let result = re.run_async(plan).await.expect("scan failed");
    assert_eq!(result.exit_status, "success");

    let docs = sink.snapshot().await;
    // Start + Descriptor + 5 × Event + Stop = 8
    assert_eq!(docs.len(), 8);

    // Descriptor should carry both motor and detector data keys.
    if let cirrus_core::Document::Descriptor(d) = &docs[1] {
        assert!(
            d.data_keys.contains_key("m1"),
            "missing motor key: {:?}",
            d.data_keys.keys().collect::<Vec<_>>()
        );
        assert!(d.data_keys.contains_key("det1_counts"));
    } else {
        panic!("doc 1 was not a Descriptor");
    }
}

#[tokio::test]
async fn fly_plan_drives_standard_detector_to_completion() {
    use cirrus::backends::soft::SoftDetector as ScalarDet;
    use cirrus_core::msg::{CollectableObj, FlyableObj, StageableObj};
    let _ = ScalarDet::new("ignored"); // ensure the import path is real

    let det = Arc::new(cirrus::backends::soft::detector::soft_detector("flydet"));
    let sink = Arc::new(CapturingSink::new());
    let re = RunEngine::new(vec![sink.clone() as Arc<dyn DocumentSink>]);

    let plan = cirrus::ophyd_async::fly(
        det.clone() as Arc<dyn FlyableObj>,
        det.clone() as Arc<dyn CollectableObj>,
        vec![det.clone() as Arc<dyn StageableObj>],
    );
    let result = re.run_async(plan).await.expect("fly failed");
    assert_eq!(result.exit_status, "success");

    let docs = sink.snapshot().await;
    // RunStart, Descriptor (from describe_collect), Event (from collect),
    // RunStop  → 4 documents minimum.
    assert!(docs.len() >= 4, "got {} docs: {:?}", docs.len(), docs);
    use cirrus_core::Document::*;
    assert!(matches!(&docs[0], Start(_)));
    assert!(matches!(&docs[docs.len() - 1], Stop(_)));
}

#[tokio::test]
async fn sync_facade_runs_blocking_count() {
    use std::thread;

    let det = SoftDetector::new("det_sync");
    let sink = Arc::new(CapturingSink::new());
    let re = Arc::new(RunEngine::new(vec![sink.clone() as Arc<dyn DocumentSink>]));

    // run_blocking must be called from a sync context (NOT inside an async task).
    let re_clone = re.clone();
    let det_clone = det.clone();
    let join = thread::spawn(move || {
        let plan = cirrus::ophyd::count(vec![det_clone], 3);
        re_clone.run_blocking(plan).unwrap()
    });
    let result = join.join().unwrap();
    assert_eq!(result.exit_status, "success");

    let docs = sink.snapshot().await;
    // Start + Descriptor + 3 × Event + Stop = 6
    assert_eq!(docs.len(), 6);
}
