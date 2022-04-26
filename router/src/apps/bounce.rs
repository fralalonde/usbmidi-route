use midi::{MidiError};
use crate::{route};
use alloc::sync::Arc;
use runtime::ExtU32;
use runtime::SpinMutex;

pub struct Bounce {
    state: Arc<SpinMutex<InnerState>>,
}

#[derive(Debug)]
struct InnerState {
    counter: u32,
}

impl InnerState {}

impl route::Service for Bounce {
    fn start(&mut self) -> Result<(), MidiError> {
        let state = self.state.clone();
        runtime::spawn(async move {
            loop {
                let mut state = state.lock();
                // midisplay::spawn(format!("{}", state.counter)).unwrap();
                state.counter += 1;
                if runtime::delay(1000.millis()).await.is_err() {break}
            }
        });

        info!("Bounce Active");
        Ok(())
    }
}
