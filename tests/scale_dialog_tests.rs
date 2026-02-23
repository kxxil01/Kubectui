//! Integration tests for the Scale Dialog component

use kubectui::ui::components::{ScaleDialogState, ScaleField, ScaleAction};

#[test]
fn test_scale_dialog_state_creation() {
    let state = ScaleDialogState::new("nginx", "default", 3);
    assert_eq!(state.deployment_name, "nginx");
    assert_eq!(state.namespace, "default");
    assert_eq!(state.current_replicas, 3);
}

#[test]
fn test_increment_decrement_logic() {
    let mut state = ScaleDialogState::new("app", "prod", 5);
    
    // Test increment
    state.handle_action(ScaleAction::Increment);
    assert_eq!(state.input_buffer, "6");
    
    // Test decrement
    state.handle_action(ScaleAction::Decrement);
    state.handle_action(ScaleAction::Decrement);
    assert_eq!(state.input_buffer, "4");
}

#[test]
fn test_digit_input() {
    let mut state = ScaleDialogState::new("web", "dev", 1);
    
    state.handle_action(ScaleAction::AddChar('2'));
    state.handle_action(ScaleAction::AddChar('5'));
    assert_eq!(state.input_buffer, "25");
    assert!(state.error_message.is_none());
}

#[test]
fn test_validation_range() {
    let mut state = ScaleDialogState::new("api", "test", 5);
    
    // Test valid range
    state.input_buffer = "50".to_string();
    state.validate_and_update();
    assert!(state.error_message.is_none());
    
    // Test above max
    state.input_buffer = "101".to_string();
    state.validate_and_update();
    assert!(state.error_message.is_some());
}

#[test]
fn test_warning_for_large_jump() {
    let mut state = ScaleDialogState::new("db", "prod", 5);
    
    state.handle_action(ScaleAction::AddChar('8'));
    state.handle_action(ScaleAction::AddChar('0'));
    
    assert!(state.warning_message.is_some());
    assert!(state.warning_message.as_ref().unwrap().contains("75"));
}

#[test]
fn test_field_focus_cycling() {
    let mut state = ScaleDialogState::new("cache", "staging", 2);
    
    assert_eq!(state.focus_field, ScaleField::InputField);
    
    state.next_field();
    assert_eq!(state.focus_field, ScaleField::ApplyBtn);
    
    state.next_field();
    assert_eq!(state.focus_field, ScaleField::CancelBtn);
    
    state.next_field();
    assert_eq!(state.focus_field, ScaleField::InputField);
}

#[test]
fn test_is_valid_check() {
    let mut state = ScaleDialogState::new("worker", "prod", 10);
    
    // Empty is invalid
    assert!(!state.is_valid());
    
    // Valid value
    state.input_buffer = "42".to_string();
    assert!(state.is_valid());
    
    // Leading zero is invalid
    state.input_buffer = "05".to_string();
    assert!(!state.is_valid());
    
    // Single zero is valid
    state.input_buffer = "0".to_string();
    assert!(state.is_valid());
}

#[test]
fn test_submit_updates_desired_replicas() {
    let mut state = ScaleDialogState::new("service", "dev", 3);
    
    state.input_buffer = "10".to_string();
    state.validate_and_update();
    state.handle_action(ScaleAction::Submit);
    
    assert_eq!(state.desired_replicas, "10");
}

#[test]
fn test_pending_flag() {
    let mut state = ScaleDialogState::new("app", "test", 1);
    
    assert!(!state.pending);
    state.set_pending(true);
    assert!(state.pending);
    state.set_pending(false);
    assert!(!state.pending);
}

#[test]
fn test_decrement_at_zero_boundary() {
    let mut state = ScaleDialogState::new("minimal", "edge", 0);
    
    state.handle_action(ScaleAction::Decrement);
    assert_eq!(state.input_buffer, "0");
    
    // Try decrementing again
    state.handle_action(ScaleAction::Decrement);
    assert_eq!(state.input_buffer, "0");
}

#[test]
fn test_increment_at_max_boundary() {
    let mut state = ScaleDialogState::new("maxed", "prod", 100);
    
    state.handle_action(ScaleAction::Increment);
    assert_eq!(state.input_buffer, "100");
    
    // Try incrementing again
    state.handle_action(ScaleAction::Increment);
    assert_eq!(state.input_buffer, "100");
}
