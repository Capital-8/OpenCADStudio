//! Upload through the queue, avoiding direct mapping of a failed allocation.

use iced::wgpu;

/// Create an unmapped buffer and upload its contents, adding COPY_DST usage.
pub fn upload_buffer<T: bytemuck::Pod>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    data: &[T],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    let bytes: &[u8] = bytemuck::cast_slice(data);
    upload_bytes(device, queue, label, bytes, std::mem::size_of::<T>(), usage)
}

/// `upload_buffer` for data already flattened to bytes. `min_size` is the
/// placeholder size used when `bytes` is empty.
pub fn upload_bytes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    bytes: &[u8],
    min_size: usize,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    const ALIGN: usize = wgpu::COPY_BUFFER_ALIGNMENT as usize;
    let logical = bytes.len().max(min_size).max(ALIGN);
    let size = logical.div_ceil(ALIGN) * ALIGN;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size as u64,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    if !bytes.is_empty() {
        if bytes.len().is_multiple_of(ALIGN) {
            queue.write_buffer(&buffer, 0, bytes);
        } else {
            let mut padded = bytes.to_vec();
            padded.resize(bytes.len().div_ceil(ALIGN) * ALIGN, 0);
            queue.write_buffer(&buffer, 0, &padded);
        }
    }
    buffer
}

/// Allocate `size` bytes and write `data` into the front of it.
///
/// For buffers deliberately larger than their initial contents — the wire
/// arena reserves headroom so later edits patch in place instead of
/// reallocating. Same no-mapping guarantee as [`upload_buffer`]; `usage` must
/// already contain `COPY_DST` for the later patches, and it is added here
/// regardless.
pub fn alloc_with_prefix<T: bytemuck::Pod>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    size: u64,
    data: &[T],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    const ALIGN: u64 = wgpu::COPY_BUFFER_ALIGNMENT;
    let size = size.max(ALIGN).div_ceil(ALIGN) * ALIGN;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bytes: &[u8] = bytemuck::cast_slice(data);
    if !bytes.is_empty() {
        let unit = ALIGN as usize;
        if bytes.len().is_multiple_of(unit) {
            queue.write_buffer(&buffer, 0, bytes);
        } else {
            let mut padded = bytes.to_vec();
            padded.resize(bytes.len().div_ceil(unit) * unit, 0);
            queue.write_buffer(&buffer, 0, &padded);
        }
    }
    buffer
}

/// Create an unmapped buffer for device-local staging: allocate `size` bytes
/// with `COPY_DST | COPY_SRC` usage, then defer the data upload to the
/// caller. Returns the buffer; the caller is responsible for uploading via
/// a [`BatchedWriter`] or direct `queue.write_buffer`.
pub fn create_empty_buffer(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    const ALIGN: u64 = wgpu::COPY_BUFFER_ALIGNMENT;
    let size = size.max(ALIGN).div_ceil(ALIGN) * ALIGN;
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// A deferred buffer writer that batches `write_buffer` calls and submits
/// them through a single encoder pass, reducing driver entry-point overhead
/// when many small writes target the same buffer or when uploads happen
/// across multiple viewport slots in a single frame.
pub struct BatchedWriter {
    /// (buffer_ptr, offset, data) triples queued for submission.
    writes: Vec<(*const wgpu::Buffer, u64, Vec<u8>)>,
}

impl BatchedWriter {
    pub fn new() -> Self {
        Self { writes: Vec::new() }
    }

    /// Queue a write to `buffer` at `offset`. The buffer must outlive the
    /// `BatchedWriter`. Multiple writes to the same buffer are submitted
    /// in submission order.
    pub fn write_buffer(&mut self, buffer: &wgpu::Buffer, offset: u64, bytes: &[u8]) {
        const ALIGN: usize = wgpu::COPY_BUFFER_ALIGNMENT as usize;
        let data = if bytes.len().is_multiple_of(ALIGN) {
            bytes.to_vec()
        } else {
            let mut padded = bytes.to_vec();
            padded.resize(bytes.len().div_ceil(ALIGN) * ALIGN, 0);
            padded
        };
        let ptr = std::ptr::from_ref(buffer);
        self.writes.push((ptr, offset, data));
    }

    /// Submit all queued writes through a single `queue.write_buffer` pass.
    /// This is equivalent to calling `write_buffer` immediately for each,
    /// but groups the work so future optimizations (e.g. staging buffer
    /// copies via `encoder.copy_buffer_to_buffer`) can be applied uniformly.
    pub fn submit(self, queue: &wgpu::Queue) {
        // SAFETY: All buffer pointers were obtained from `std::ptr::from_ref`
        // on `&wgpu::Buffer` references that outlive this call. The buffers
        // are not modified or destroyed during this loop.
        for (ptr, offset, data) in self.writes {
            let buffer = unsafe { &*ptr };
            queue.write_buffer(buffer, offset, &data);
        }
    }

    /// True when no writes have been queued.
    pub fn is_empty(&self) -> bool {
        self.writes.is_empty()
    }

    /// Number of queued write operations.
    pub fn len(&self) -> usize {
        self.writes.len()
    }
}

impl Default for BatchedWriter {
    fn default() -> Self {
        Self::new()
    }
}

/// Buffer + writer combo: create the buffer and stage its initial data
/// in the batched writer instead of writing immediately. Use when you
/// know all the data up front and want to defer the actual GPU transfer
/// to a single `submit` call.
pub fn create_buffer_batched<T: bytemuck::Pod>(
    device: &wgpu::Device,
    writer: &mut BatchedWriter,
    label: &str,
    data: &[T],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    let bytes: &[u8] = bytemuck::cast_slice(data);
    const ALIGN: usize = wgpu::COPY_BUFFER_ALIGNMENT as usize;
    let logical = bytes.len().max(ALIGN);
    let size = logical.div_ceil(ALIGN) * ALIGN;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size as u64,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    if !bytes.is_empty() {
        writer.write_buffer(&buffer, 0, bytes);
    }
    buffer
}
