//! Comprehensive tests for scaling operations - Stream C

use kubectui::k8s::scaling::{ScaleError, ScaleProgress, ScaleRequest};
use std::error::Error;

#[test]
fn test_scale_request_validation_passes_zero() {
    let req = ScaleRequest::new("deploy", "default", 0);
    assert!(req.validate().is_ok());
}

#[test]
fn test_scale_request_validation_passes_one() {
    let req = ScaleRequest::new("deploy", "default", 1);
    assert!(req.validate().is_ok());
}

#[test]
fn test_scale_request_validation_passes_hundred() {
    let req = ScaleRequest::new("deploy", "default", 100);
    assert!(req.validate().is_ok());
}

#[test]
fn test_scale_request_validation_fails_negative() {
    let req = ScaleRequest::new("deploy", "default", -1);
    assert!(req.validate().is_err());
}

#[test]
fn test_scale_request_validation_fails_over_limit() {
    let req = ScaleRequest::new("deploy", "default", 101);
    assert!(req.validate().is_err());
}

#[test]
fn test_scale_error_deployment_not_found_display() {
    let err = ScaleError::DeploymentNotFound("nginx".to_string(), "production".to_string());
    let display_msg = format!("{}", err);
    assert!(display_msg.contains("nginx"));
    assert!(display_msg.contains("production"));
}

#[test]
fn test_scale_error_invalid_replica_count_display() {
    let err = ScaleError::InvalidReplicaCount(150);
    let display_msg = format!("{}", err);
    assert!(display_msg.contains("150"));
    assert!(display_msg.contains("Invalid"));
}

#[test]
fn test_scale_error_api_error_display() {
    let err = ScaleError::ApiError("connection refused".to_string());
    let display_msg = format!("{}", err);
    assert!(display_msg.contains("connection refused"));
}

#[test]
fn test_scale_error_timeout_display() {
    let err = ScaleError::Timeout("waited 120 seconds".to_string());
    let display_msg = format!("{}", err);
    assert!(display_msg.contains("Timeout"));
}

#[test]
fn test_scale_error_cancelled_display() {
    let err = ScaleError::Cancelled;
    let display_msg = format!("{}", err);
    assert!(display_msg.contains("cancelled"));
}

#[test]
fn test_scale_error_implements_error_trait() {
    let err: Box<dyn Error> = Box::new(ScaleError::Cancelled);
    assert_eq!(err.to_string(), "Scale operation cancelled");
}

#[test]
fn test_scale_progress_initiated_serialization() {
    let progress = ScaleProgress::Initiated;
    let json = serde_json::to_string(&progress).expect("should serialize");
    assert!(json.contains("Initiated"));

    let deserialized: ScaleProgress = serde_json::from_str(&json).expect("should deserialize");
    assert_eq!(format!("{:?}", deserialized), "Initiated");
}

#[test]
fn test_scale_progress_scaling_serialization() {
    let progress = ScaleProgress::Scaling {
        current: 2,
        target: 5,
    };
    let json = serde_json::to_string(&progress).expect("should serialize");
    assert!(json.contains("2"));
    assert!(json.contains("5"));
}

#[test]
fn test_scale_progress_success_serialization() {
    let progress = ScaleProgress::Success {
        current: 5,
        target: 5,
    };
    let json = serde_json::to_string(&progress).expect("should serialize");
    assert!(json.contains("5"));
}

#[test]
fn test_scale_progress_error_serialization() {
    let progress = ScaleProgress::Error("API error".to_string());
    let json = serde_json::to_string(&progress).expect("should serialize");
    assert!(json.contains("API error"));
}

#[test]
fn test_scale_request_with_various_namespaces() {
    let req1 = ScaleRequest::new("deploy", "default", 1);
    assert_eq!(req1.namespace, "default");

    let req2 = ScaleRequest::new("deploy", "kube-system", 1);
    assert_eq!(req2.namespace, "kube-system");
}

#[test]
fn test_scale_request_with_various_deployment_names() {
    let req1 = ScaleRequest::new("nginx", "default", 1);
    assert_eq!(req1.deployment, "nginx");

    let req2 = ScaleRequest::new("my-api-server", "default", 1);
    assert_eq!(req2.deployment, "my-api-server");
}

#[test]
fn test_scale_request_boundary_values() {
    let req_min = ScaleRequest::new("deploy", "default", 0);
    assert!(req_min.validate().is_ok());

    let req_mid = ScaleRequest::new("deploy", "default", 50);
    assert!(req_mid.validate().is_ok());

    let req_max = ScaleRequest::new("deploy", "default", 100);
    assert!(req_max.validate().is_ok());
}

#[test]
fn test_scale_error_clone() {
    let err1 = ScaleError::Cancelled;
    let err2 = err1.clone();
    assert_eq!(format!("{}", err1), format!("{}", err2));
}

#[test]
fn test_scale_progress_clone() {
    let progress = ScaleProgress::Scaling {
        current: 2,
        target: 5,
    };
    let cloned = progress.clone();
    if let ScaleProgress::Scaling { current, target } = cloned {
        assert_eq!(current, 2);
        assert_eq!(target, 5);
    } else {
        panic!("Clone failed");
    }
}
