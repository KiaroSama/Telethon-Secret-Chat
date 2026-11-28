mod immediate;
use super::decoder::{ColorTransform, choose_color_convert_func};
use super::error::Result;
use super::parser::{Component, Dimensions};
use super::upsampler::Upsampler;
use std::{cell::RefCell, sync::Arc};
pub struct RowData {
    pub index: usize,
    pub component: Component,
    pub quantization_table: Arc<[u16; 64]>,
}
pub trait Worker {
    fn start(&mut self, row_data: RowData) -> Result<()>;
    fn append_row(&mut self, row: (usize, Vec<i16>)) -> Result<()>;
    fn get_result(&mut self, index: usize) -> Result<Vec<u8>>;
    fn append_rows(&mut self, rows: &mut dyn Iterator<Item = (usize, Vec<i16>)>) -> Result<()> {
        for row in rows {
            self.append_row(row)?;
        }
        Ok(())
    }
}
#[derive(Default)]
pub struct WorkerScope {
    inner: RefCell<immediate::ImmediateWorker>,
}
impl WorkerScope {
    pub fn with<T>(with: impl FnOnce(&Self) -> T) -> T {
        with(&Self::default())
    }
    pub fn get_or_init_worker<T>(&self, f: impl FnOnce(&mut dyn Worker) -> T) -> T {
        f(&mut *self.inner.borrow_mut())
    }
}
pub fn compute_image_portable(
    components: &[Component],
    data: Vec<Vec<u8>>,
    size: Dimensions,
    transform: ColorTransform,
) -> Result<Vec<u8>> {
    let convert = choose_color_convert_func(components.len(), transform)?;
    let upsampler = Upsampler::new(components, size.width, size.height)?;
    let line_size = usize::from(size.width) * components.len();
    let mut image = vec![0; line_size * usize::from(size.height)];
    for (row, line) in image.chunks_mut(line_size).enumerate() {
        upsampler.upsample_and_interleave_row(&data, row, usize::from(size.width), line, convert);
    }
    Ok(image)
}
