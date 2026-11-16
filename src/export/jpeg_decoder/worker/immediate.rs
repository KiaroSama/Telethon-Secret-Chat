use super::super::decoder::MAX_COMPONENTS;
use super::super::error::{Error, Result};
use super::super::idct::dequantize_and_idct_block;
use super::super::parser::Component;
use super::{RowData, Worker};
use core::mem;
use std::sync::Arc;
use std::vec;
use std::vec::Vec;

pub struct ImmediateWorker {
    offsets: [usize; MAX_COMPONENTS],
    results: Vec<Vec<u8>>,
    components: Vec<Option<Component>>,
    quantization_tables: Vec<Option<Arc<[u16; 64]>>>,
}

impl Default for ImmediateWorker {
    fn default() -> Self {
        ImmediateWorker {
            offsets: [0; MAX_COMPONENTS],
            results: vec![Vec::new(); MAX_COMPONENTS],
            components: vec![None; MAX_COMPONENTS],
            quantization_tables: vec![None; MAX_COMPONENTS],
        }
    }
}

impl ImmediateWorker {
    pub fn start_immediate(&mut self, data: RowData) {
        assert!(self.results[data.index].is_empty());

        self.offsets[data.index] = 0;
        self.results[data.index].resize(
            data.component.block_size.width as usize
                * data.component.block_size.height as usize
                * data.component.dct_scale
                * data.component.dct_scale,
            0u8,
        );
        self.components[data.index] = Some(data.component);
        self.quantization_tables[data.index] = Some(data.quantization_table);
    }

    pub fn append_row_immediate(&mut self, (index, data): (usize, Vec<i16>)) -> Result<()> {
        // Convert coefficients from a MCU row to samples.

        let component = self
            .components
            .get(index)
            .and_then(Option::as_ref)
            .ok_or_else(|| Error::Format("missing component".into()))?;
        let quantization_table = self
            .quantization_tables
            .get(index)
            .and_then(Option::as_ref)
            .ok_or_else(|| Error::Format("missing quantization table".into()))?;
        let block_count =
            component.block_size.width as usize * component.vertical_sampling_factor as usize;
        let line_stride = component.block_size.width as usize * component.dct_scale;

        if data.len() != block_count * 64 {
            return Err(Error::Format("invalid coefficient count".into()));
        }

        for i in 0..block_count {
            let x = (i % component.block_size.width as usize) * component.dct_scale;
            let y = (i / component.block_size.width as usize) * component.dct_scale;

            let coefficients = data[i * 64..(i + 1) * 64]
                .try_into()
                .map_err(|_| Error::Format("invalid coefficient block".into()))?;
            let output = &mut self.results[index][self.offsets[index] + y * line_stride + x..];

            dequantize_and_idct_block(
                component.dct_scale,
                coefficients,
                quantization_table,
                line_stride,
                output,
            );
        }

        self.offsets[index] += block_count * component.dct_scale * component.dct_scale;
        Ok(())
    }

    pub fn get_result_immediate(&mut self, index: usize) -> Vec<u8> {
        mem::take(&mut self.results[index])
    }
}

impl Worker for ImmediateWorker {
    fn start(&mut self, data: RowData) -> Result<()> {
        self.start_immediate(data);
        Ok(())
    }
    fn append_row(&mut self, row: (usize, Vec<i16>)) -> Result<()> {
        self.append_row_immediate(row)
    }
    fn get_result(&mut self, index: usize) -> Result<Vec<u8>> {
        Ok(self.get_result_immediate(index))
    }
}
