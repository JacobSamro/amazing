//! CPU-backed GPU process. The child owns retained resources and presents a
//! validated compositor frame reconstructed from its versioned snapshot.
use crate::gpu_snapshot_wire::{decode_snapshot, encode_snapshot};
use crate::protocol::{Frame, GpuFrame, GpuPresentation, GpuReceiver};
use amazing_compositor::{CompositorFrame, Presenter, ReferenceCpuPresenter};
use amazing_ipc::codec::{self, ByteBuf};
use amazing_ipc::{Connection, Error};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

const MAX_RETAINED_RESOURCE_BYTES: usize = 256 * 1024 * 1024;

impl GpuFrame {
    /// Preserve passes, quads, property trees, tile ids, and resource deltas.
    pub fn from_compositor(sequence: u64, frame: &CompositorFrame) -> Result<Self, Error> {
        let snapshot = frame.to_snapshot();
        let bytes = encode_snapshot(&snapshot)?;
        Ok(Self {
            sequence,
            snapshot: ByteBuf(bytes),
        })
    }
}

#[derive(Default)]
struct PresenterState {
    presenter: ReferenceCpuPresenter,
    sizes: BTreeMap<u64, usize>,
}
impl PresenterState {
    fn present(&mut self, request: GpuFrame) -> Result<Frame, Error> {
        let snapshot = decode_snapshot(&request.snapshot.0)?;
        let frame = CompositorFrame::from_snapshot(snapshot)
            .map_err(|e| Error::Remote(format!("invalid compositor snapshot: {e}")))?;
        let output_len = (frame.width as usize)
            .checked_mul(frame.height as usize)
            .and_then(|n| n.checked_mul(4))
            .filter(|n| *n <= codec::MAX_FRAME - 1024)
            .ok_or(Error::Codec(codec::Error::Limit))?;
        let evictions: BTreeSet<u64> = frame.resources.evictions.iter().map(|id| id.0).collect();
        let uploads: BTreeSet<u64> = frame.resources.uploads.iter().map(|u| u.id.0).collect();
        let retained = self
            .sizes
            .iter()
            .filter(|(id, _)| !evictions.contains(id) && !uploads.contains(id))
            .try_fold(0usize, |total, (_, size)| total.checked_add(*size))
            .ok_or(Error::Codec(codec::Error::Limit))?;
        let total = frame
            .resources
            .uploads
            .iter()
            .try_fold(retained, |total, upload| {
                total.checked_add(upload.pixels.len())
            })
            .ok_or(Error::Codec(codec::Error::Limit))?;
        if total > MAX_RETAINED_RESOURCE_BYTES {
            return Err(Error::Codec(codec::Error::Limit));
        }
        let surface = self
            .presenter
            .present(&frame)
            .map_err(|e| Error::Remote(format!("GPU presenter: {e}")))?;
        debug_assert_eq!(surface.pixels.len(), output_len);
        for id in evictions {
            self.sizes.remove(&id);
        }
        for upload in &frame.resources.uploads {
            self.sizes.insert(upload.id.0, upload.pixels.len());
        }
        Ok(Frame {
            sequence: request.sequence,
            width: frame.width,
            height: frame.height,
            rgba: ByteBuf(surface.pixels),
        })
    }
}
#[derive(Default)]
struct Endpoint(Mutex<PresenterState>);
impl GpuPresentation for Endpoint {
    fn submit(&self, frame: GpuFrame) -> Result<Frame, Error> {
        self.0.lock().unwrap().present(frame)
    }
}
pub fn serve_gpu(connection: &Arc<Connection>) {
    connection.bind(11, GpuReceiver(Endpoint::default()));
    connection.bind(0, crate::Ping);
}
