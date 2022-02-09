use log::{error, info};
use rusb::{self, DeviceHandle, GlobalContext};
use std::{
  error::Error,
  mem::swap,
  ops::{Deref, DerefMut},
  time::Duration,
};

use crate::slider_io::{
  config::HardwareSpec,
  controller_state::{ControllerState, FullState, LedState},
  utils::{Buffer, ShimError},
  worker::ThreadJob,
};

type HidReadCallback = fn(&Buffer, &mut ControllerState) -> ();
type HidLedCallback = fn(&mut Buffer, &LedState) -> ();

enum WriteType {
  Bulk,
  Interrupt,
}

pub struct HidDeviceJob {
  state: FullState,

  vid: u16,
  pid: u16,
  read_endpoint: u8,
  led_endpoint: u8,

  read_callback: HidReadCallback,
  read_buf: Buffer,
  last_read_buf: Buffer,

  led_write_type: WriteType,
  led_callback: HidLedCallback,
  led_buf: Buffer,

  handle: Option<DeviceHandle<GlobalContext>>,
}

impl HidDeviceJob {
  fn new(
    state: FullState,
    vid: u16,
    pid: u16,
    read_endpoint: u8,
    led_endpoint: u8,
    read_callback: HidReadCallback,
    led_type: WriteType,
    led_callback: HidLedCallback,
  ) -> Self {
    Self {
      state,
      vid,
      pid,
      read_endpoint,
      led_endpoint,
      read_callback,
      read_buf: Buffer::new(),
      last_read_buf: Buffer::new(),
      led_write_type: led_type,
      led_callback,
      led_buf: Buffer::new(),
      handle: None,
    }
  }

  pub fn from_config(state: &FullState, spec: &HardwareSpec) -> Self {
    match spec {
      HardwareSpec::TasollerOne => Self::new(
        state.clone(),
        0x1ccf,
        0x2333,
        0x84,
        0x03,
        |buf, controller_state| {
          if buf.len != 11 {
            return;
          }

          let bits: Vec<u8> = buf
            .data
            .iter()
            .flat_map(|x| (0..8).map(move |i| ((x) >> i) & 1))
            .collect();
          for i in 0..32 {
            controller_state.ground_state[i] = bits[34 + i] * 255;
          }
          controller_state.flip_vert();

          controller_state.air_state.copy_from_slice(&bits[28..34]);
          controller_state.extra_state[0..2].copy_from_slice(&bits[26..28]);
        },
        WriteType::Bulk,
        |buf, led_state| {
          buf.len = 240;
          buf.data[0] = 'B' as u8;
          buf.data[1] = 'L' as u8;
          buf.data[2] = '\x00' as u8;
          for (buf_chunk, state_chunk) in buf.data[3..96]
            .chunks_mut(3)
            .take(31)
            .zip(led_state.led_state.chunks(3).rev())
          {
            buf_chunk[0] = state_chunk[1];
            buf_chunk[1] = state_chunk[0];
            buf_chunk[2] = state_chunk[2];
          }
          buf.data[96..240].fill(0);
        },
      ),
      HardwareSpec::TasollerTwo => Self::new(
        state.clone(),
        0x1ccf,
        0x2333,
        0x84,
        0x03,
        |buf, controller_state| {
          if buf.len != 36 {
            return;
          }

          controller_state
            .ground_state
            .copy_from_slice(&buf.data[4..36]);
          controller_state.flip_vert();

          let bits: Vec<u8> = (0..8).map(|x| (buf.data[3] >> x) & 1).collect();
          controller_state.air_state.copy_from_slice(&bits[0..6]);
          controller_state.extra_state[0..2].copy_from_slice(&bits[6..8]);
        },
        WriteType::Bulk,
        |buf, led_state| {
          buf.len = 240;
          buf.data[0] = 'B' as u8;
          buf.data[1] = 'L' as u8;
          buf.data[2] = '\x00' as u8;
          for (buf_chunk, state_chunk) in buf.data[3..96]
            .chunks_mut(3)
            .take(31)
            .zip(led_state.led_state.chunks(3).rev())
          {
            buf_chunk[0] = state_chunk[1];
            buf_chunk[1] = state_chunk[0];
            buf_chunk[2] = state_chunk[2];
          }
          buf.data[96..240].fill(0);
        },
      ),
      HardwareSpec::Yuancon => Self::new(
        state.clone(),
        0x1973,
        0x2001,
        0x81,
        0x02,
        |buf, controller_state| {
          if buf.len != 34 {
            return;
          }

          controller_state
            .ground_state
            .copy_from_slice(&buf.data[2..34]);
          for i in 0..6 {
            controller_state.air_state[i ^ 1] = (buf.data[0] >> i) & 1;
          }
          for i in 0..3 {
            controller_state.extra_state[2 - i] = (buf.data[1] >> i) & 1;
          }
        },
        WriteType::Interrupt,
        |buf, led_state| {
          buf.len = 31 * 2;
          for (buf_chunk, state_chunk) in buf
            .data
            .chunks_mut(2)
            .take(31)
            .zip(led_state.led_state.chunks(3).rev())
          {
            buf_chunk[0] = (state_chunk[0] << 3 & 0xe0) | (state_chunk[2] >> 3);
            buf_chunk[1] = (state_chunk[1] & 0xf8) | (state_chunk[0] >> 5);
          }
        },
      ),
    }
  }

  fn get_handle(&mut self) -> Result<(), Box<dyn Error>> {
    info!("Device finding vid {} pid {}", self.vid, self.pid);
    let handle = rusb::open_device_with_vid_pid(self.vid, self.pid);
    if handle.is_none() {
      error!("Device not found");
      return Err(Box::new(ShimError));
    }
    let mut handle = handle.unwrap();
    info!("Device found {:?}", handle);

    if handle.kernel_driver_active(0).unwrap_or(false) {
      info!("Device detaching kernel driver");
      handle.detach_kernel_driver(0)?;
    }
    info!("Device setting configuration");
    handle.set_active_configuration(1)?;
    info!("Device claiming interface");
    handle.claim_interface(0)?;
    self.handle = Some(handle);
    Ok(())
  }
}

const TIMEOUT: Duration = Duration::from_millis(20);

impl ThreadJob for HidDeviceJob {
  fn setup(&mut self) -> bool {
    match self.get_handle() {
      Ok(_) => {
        info!("Device OK");
        true
      }
      Err(e) => {
        error!("Device setup failed: {}", e);
        false
      }
    }
  }

  fn tick(&mut self) -> bool {
    // Input loop
    let handle = self.handle.as_mut().unwrap();
    let mut work = false;

    {
      let res = handle
        .read_interrupt(self.read_endpoint, &mut self.read_buf.data, TIMEOUT)
        .map_err(|e| {
          // debug!("Device read error {}", &e);
          e
        })
        .unwrap_or(0);
      self.read_buf.len = res;
      // debug!("{:?}", self.read_buf.slice());
      // if self.read_buf.len != 0 {
      if (self.read_buf.len != 0) && (self.read_buf.slice() != self.last_read_buf.slice()) {
        work = true;
        let mut controller_state_handle = self.state.controller_state.lock();
        (self.read_callback)(&self.read_buf, controller_state_handle.deref_mut());
        swap(&mut self.read_buf, &mut self.last_read_buf);
      }
    }

    // Led loop
    {
      {
        let mut led_state_handle = self.state.led_state.lock();
        if led_state_handle.dirty {
          (self.led_callback)(&mut self.led_buf, led_state_handle.deref());
          led_state_handle.dirty = false;
        }
      }

      if self.led_buf.len != 0 {
        let res = (match self.led_write_type {
          WriteType::Bulk => handle.write_bulk(self.led_endpoint, self.led_buf.slice(), TIMEOUT),
          WriteType::Interrupt => {
            handle.write_interrupt(self.led_endpoint, &self.led_buf.slice(), TIMEOUT)
          }
        })
        .map_err(|e| {
          // debug!("Device write error {}", e);
          e
        })
        .unwrap_or(0);
        if res == self.led_buf.len + 1 {
          // work = true;
          self.led_buf.len = 0;
        }
      }
    }

    work
  }
}

impl Drop for HidDeviceJob {
  fn drop(&mut self) {
    if let Some(handle) = self.handle.as_mut() {
      handle.release_interface(0).ok();
    }
  }
}
