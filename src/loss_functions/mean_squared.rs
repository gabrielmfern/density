//! The module that implements the Mean Squared loss function.

use std::mem;

use opencl3::{
    device::cl_float,
    error_codes::{cl_int, ClError},
    kernel::ExecuteKernel,
    memory::{Buffer, ClMem, CL_MEM_READ_WRITE},
};

use crate::loss_functions::LossFunction;
use crate::types::ModelLossFunction;
use crate::utils::opencl::empty_buffer;
use crate::utils::opencl::ensure_program;
use crate::utils::opencl::EnsureKernelsAndProgramError;
use crate::utils::BufferOperations;
use crate::utils::OpenCLState;

use super::{LossComputationError, LossToModelOutputsDerivativesComputationError};

const PROGRAM_NAME: &str = "MEAN_SQUARED";
const PROGRAM_SOURCE: &str = include_str!("kernels/mean_squared.cl");
const COMPUTE_LOSS_KERNEL: &str = "compute_loss";
const COMPUTE_LOSS_TO_OUTPUT_DERIVATIVES_KERNEL: &str = "compute_loss_to_output_derivatives";

pub(crate) fn compile_mean_squared(
    opencl_state: &mut OpenCLState,
) -> Result<(), EnsureKernelsAndProgramError> {
    let kernels = &[
        COMPUTE_LOSS_KERNEL.to_string(),
        COMPUTE_LOSS_TO_OUTPUT_DERIVATIVES_KERNEL.to_string(),
    ];

    ensure_program(
        opencl_state,
        PROGRAM_NAME.to_string(),
        PROGRAM_SOURCE.to_string(),
        "".to_string(),
        kernels,
    )?;

    Ok(())
}

#[derive(Debug)]
/// The Mean Squared loss function, good for some problem with
/// linear regression, because this error is quite free, in comparison
/// to the `Categorical Cross Entropy` loss function which restricts things
/// to be in (0, 1) (a **closed interval** between 0 and 1) to work well.
pub struct MeanSquared<'a> {
    opencl_state: Option<&'a OpenCLState>,
}

impl<'a> MeanSquared<'a> {
    /// Creates a new instance of the Mean Squared but as a ModelLossFunction variant
    /// for using in the **TrainingOptions** when fitting a Model.
    ///
    /// Be aware that after creation this needs to be called the `init` method before computing the
    /// loss or anything like that.`
    /// But when it is being used a Model, the Model will call the init automatically.`
    pub fn new() -> ModelLossFunction<'a> {
        Self::new_raw().into()
    }

    /// Crates a new instance of the Mean Squared but as a raw version of the struct.
    ///
    /// Be aware that after creation this needs to be called the `init` method before computing the
    /// loss or anything like that.`
    pub fn new_raw() -> MeanSquared<'a> {
        MeanSquared { opencl_state: None }
    }
}

impl<'a> LossFunction<'a> for MeanSquared<'a> {
    fn init(&mut self, opencl_state: &'a OpenCLState) -> Result<(), ClError> {
        self.opencl_state = Some(opencl_state);

        Ok(())
    }

    fn compute_loss(
        &self,
        output_samples: &Buffer<cl_float>,
        expected_outputs: &Buffer<cl_float>,
        samples_amount: usize,
    ) -> Result<f32, LossComputationError> {
        if self.opencl_state.is_none() {
            return Err(LossComputationError::NotInitialized);
        }

        let state = self.opencl_state.unwrap();

        if state.queues.len() == 0 {
            return Err(LossComputationError::NoCommandQueue);
        }

        if output_samples.size()? != expected_outputs.size()? {
            return Err(LossComputationError::OutputsAndExpectedOutputsDoNotMatch);
        }

        let queue = state.queues.first().unwrap();

        let outputs_total_count = output_samples.size()? / mem::size_of::<cl_float>();
        if outputs_total_count % samples_amount != 0 {
            return Err(LossComputationError::TrainingDataDoesNotHaveExpectedSamplesAmount);
        }

        let outputs_amount = outputs_total_count / samples_amount;

        let sample_losses_buffer = empty_buffer(samples_amount, CL_MEM_READ_WRITE, state)?;

        let program = state.get_prgm(PROGRAM_NAME)?;

        let compute_loss_kernel = program.get_krnl(COMPUTE_LOSS_KERNEL)?;

        ExecuteKernel::new(compute_loss_kernel)
            .set_arg(output_samples)
            .set_arg(expected_outputs)
            .set_arg(&sample_losses_buffer)
            .set_arg(&(outputs_amount as cl_int))
            .set_arg(&(samples_amount as cl_int))
            .set_global_work_size(samples_amount)
            .enqueue_nd_range(queue)?
            .wait()?;

        Ok(sample_losses_buffer
            .sum(self.opencl_state.unwrap())?
            / outputs_amount as f32
            / samples_amount as f32)
    }

    fn compute_loss_derivative_with_respect_to_output_samples(
        &self,
        output_samples: &Buffer<cl_float>,
        expected_outputs: &Buffer<cl_float>,
        samples_amount: usize,
    ) -> Result<Buffer<cl_float>, LossToModelOutputsDerivativesComputationError> {
        if self.opencl_state.is_none() {
            return Err(LossToModelOutputsDerivativesComputationError::NotInitialized);
        }

        let state = self.opencl_state.unwrap();

        if state.queues.len() == 0 {
            return Err(LossToModelOutputsDerivativesComputationError::NoCommandQueue);
        }

        if output_samples.size()? != expected_outputs.size()? {
            return Err(LossToModelOutputsDerivativesComputationError::OutputsAndExpectedOutputsDoNotMatch);
        }

        let outputs_total_count = output_samples.size()? / mem::size_of::<cl_float>();
        if outputs_total_count % samples_amount != 0 {
            return Err(LossToModelOutputsDerivativesComputationError::TrainingDataDoesNotHaveExpectedSamplesAmount);
        }

        let outputs_amount = outputs_total_count / samples_amount;

        let derivatives_buffer = empty_buffer(outputs_total_count, CL_MEM_READ_WRITE, state)?;

        let program = state.get_prgm(PROGRAM_NAME)?;
        let compute_loss_to_output_derivatives_kernel = program.get_krnl(COMPUTE_LOSS_TO_OUTPUT_DERIVATIVES_KERNEL)?;

        ExecuteKernel::new(&compute_loss_to_output_derivatives_kernel)
            .set_arg(output_samples)
            .set_arg(expected_outputs)
            .set_arg(&derivatives_buffer)
            .set_arg(&(samples_amount as cl_int))
            .set_arg(&(outputs_amount as cl_int))
            .set_global_work_sizes(&[samples_amount, outputs_amount])
            .enqueue_nd_range(state.queues.first().unwrap())?
            .wait()?;

        Ok(derivatives_buffer)
    }
}

#[cfg(test)]
mod mean_squared_tests {
    use std::ptr;

    use opencl3::{
        memory::{Buffer, CL_MEM_READ_ONLY},
        types::{cl_float, CL_NON_BLOCKING},
    };
    use rand::{thread_rng, Rng};

    use super::MeanSquared;
    use crate::utils::{approx_eq::assert_approx_equal_distance, setup_opencl, OpenCLState};
    use crate::{
        loss_functions::LossFunction, utils::opencl::DeviceType,
    };

    #[test]
    fn should_compute_derivatives_up_to_a_certain_precision()
    {
        let opencl_state: OpenCLState = setup_opencl(DeviceType::GPU).unwrap();

        let mut gpu_loss = MeanSquared::new_raw();
        gpu_loss.init(&opencl_state).unwrap();

        let outputs_amount: usize = 61;
        let samples_amount: usize = 113;
        let mut rng = rand::thread_rng();

        let output_samples: Vec<f32> = (0..(samples_amount * outputs_amount))
            .into_iter()
            .map(|_| rng.gen_range(-1123.0_f32..1543_f32))
            .collect();
        let expected_outputs: Vec<f32> = (0..(samples_amount * outputs_amount))
            .into_iter()
            .map(|_| rng.gen_range(-1313.0_f32..1413_f32))
            .collect();

        let expected_derivatives: Vec<f32> = expected_outputs
            .iter()
            .zip(&output_samples)
            .map(|(expected_output, actual_output)| {
                2.0 / outputs_amount as f32 * (actual_output - expected_output)
            })
            .collect();

        let mut outputs_buf = Buffer::<cl_float>::create(
            &opencl_state.context,
            CL_MEM_READ_ONLY,
            samples_amount * outputs_amount,
            ptr::null_mut(),
        ).unwrap();
        let mut expected_outputs_buf = Buffer::<cl_float>::create(
            &opencl_state.context,
            CL_MEM_READ_ONLY,
            samples_amount * outputs_amount,
            ptr::null_mut(),
        ).unwrap();

        let queue = opencl_state.queues.first().unwrap();

        queue
            .enqueue_write_buffer(
                &mut outputs_buf,
                CL_NON_BLOCKING,
                0,
                output_samples.as_slice(),
                &[],
            ).unwrap()
            .wait().unwrap();
        queue
            .enqueue_write_buffer(
                &mut expected_outputs_buf,
                CL_NON_BLOCKING,
                0,
                expected_outputs.as_slice(),
                &[],
            ).unwrap()
            .wait().unwrap();

        let buf = gpu_loss.compute_loss_derivative_with_respect_to_output_samples(
            &outputs_buf,
            &expected_outputs_buf,
            samples_amount,
        ).unwrap();
        let mut derivatives_vec = vec![0.0; samples_amount * outputs_amount];
        let derivatives_slice = derivatives_vec.as_mut_slice();

        queue
            .enqueue_read_buffer(&buf, CL_NON_BLOCKING, 0, derivatives_slice, &[]).unwrap()
            .wait().unwrap();

        assert_approx_equal_distance(&expected_derivatives, &derivatives_vec, 0.01);
    }

    #[test]
    fn should_compute_loss_up_to_a_certain_precision() {
        let opencl_state: OpenCLState = setup_opencl(DeviceType::GPU).unwrap();

        let mut loss = MeanSquared::new();
        loss.init(&opencl_state).unwrap();

        let mut rng = thread_rng();
        let samples_amount = 27;
        let outputs_amount = 29;
        let outputs: Vec<f32> = (0..(samples_amount * outputs_amount))
            .into_iter()
            .map(|_| rng.gen_range(-1241_f32..2192_f32))
            .collect();
        let expected_outputs: Vec<f32> = (0..(samples_amount * outputs_amount))
            .into_iter()
            .map(|_| rng.gen_range(-1241_f32..2192_f32))
            .collect();

        let expected_loss: f32 = expected_outputs
            .iter()
            .zip(&outputs)
            .map(|(output, expected_output)| (output - expected_output).powf(2.0))
            .sum::<f32>()
            / outputs_amount as f32
            / samples_amount as f32;
        let mut outputs_buf = Buffer::<cl_float>::create(
            &opencl_state.context,
            CL_MEM_READ_ONLY,
            samples_amount * outputs_amount,
            ptr::null_mut(),
        ).unwrap();
        let mut expected_outputs_buf = Buffer::<cl_float>::create(
            &opencl_state.context,
            CL_MEM_READ_ONLY,
            samples_amount * outputs_amount,
            ptr::null_mut(),
        ).unwrap();

        let queue = opencl_state.queues.first().unwrap();

        queue
            .enqueue_write_buffer(
                &mut outputs_buf,
                CL_NON_BLOCKING,
                0,
                outputs.as_slice(),
                &[],
            ).unwrap()
            .wait().unwrap();
        queue
            .enqueue_write_buffer(
                &mut expected_outputs_buf,
                CL_NON_BLOCKING,
                0,
                expected_outputs.as_slice(),
                &[],
            ).unwrap()
            .wait().unwrap();

        let actual_loss = loss.compute_loss(&outputs_buf, &expected_outputs_buf, samples_amount).unwrap();

        println!(
            "|({} - {}) / {}| <= 0.1%",
            expected_loss,
            actual_loss,
            expected_loss.max(actual_loss)
        );
        assert!((expected_loss - actual_loss).abs() / expected_loss.max(actual_loss) <= 0.001);
    }
}