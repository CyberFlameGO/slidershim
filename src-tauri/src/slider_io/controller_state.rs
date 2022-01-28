use std::{
  sync::{Arc, Mutex},
  time::Instant,
};

pub struct ControllerState {
  pub ground_state: [u8; 32],
  pub air_state: [u8; 6],
  pub extra_state: [u8; 3],
}

impl ControllerState {
  pub fn new() -> Self {
    Self {
      ground_state: [0; 32],
      air_state: [0; 6],
      extra_state: [0; 3],
    }
  }
}

pub struct LedState {
  pub led_state: [u8; 3 * 31],
  pub dirty: bool,
  pub start: Instant,
}

impl LedState {
  pub fn new() -> Self {
    Self {
      led_state: [0; 3 * 31],
      dirty: false,
      start: Instant::now(),
    }
  }
}

pub struct FullState {
  pub controller_state: Arc<Mutex<ControllerState>>,
  pub led_state: Arc<Mutex<LedState>>,
}

impl FullState {
  pub fn new() -> Self {
    Self {
      controller_state: Arc::new(Mutex::new(ControllerState::new())),
      led_state: Arc::new(Mutex::new(LedState::new())),
    }
  }

  pub fn clone_controller(&self) -> Arc<Mutex<ControllerState>> {
    Arc::clone(&self.controller_state)
  }

  pub fn clone_led(&self) -> Arc<Mutex<LedState>> {
    Arc::clone(&self.led_state)
  }
}
