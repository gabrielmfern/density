use crate::{layers::Layer, types::CompilationOrOpenCLError};

use opencl3::memory::ClMem;
use opencl3::{
    command_queue::CommandQueue,
    context::Context,
    device::cl_float,
    error_codes::{cl_int, ClError},
    kernel::{ExecuteKernel, Kernel},
    memory::{Buffer, CL_MEM_READ_ONLY, CL_MEM_READ_WRITE},
    program::Program,
};
use std::mem;
use std::ptr;

use savefile_derive::Savefile;

const PROGRAM_SOURCE: &str = include_str!("kernels/tanh.cl");
const PROPAGATE_KERNEL_NAME: &str = "propagate";
const BACK_PROPAGATE_KERNEL_NAME: &str = "back_propagate";

#[derive(Debug, Savefile)]
pub struct TanH<'a> {
    pub inputs_amount: usize,

    #[savefile_ignore]
    #[savefile_introspect_ignore]
    pub last_inputs_buffer: Option<Buffer<cl_float>>,
    #[savefile_ignore]
    #[savefile_introspect_ignore]
    pub last_outputs_buffer: Option<Buffer<cl_float>>,

    #[savefile_ignore]
    #[savefile_introspect_ignore]
    pub opencl_context: Option<&'a Context>,
    #[savefile_ignore]
    #[savefile_introspect_ignore]
    pub opencl_queue: Option<&'a CommandQueue>,

    #[savefile_ignore]
    #[savefile_introspect_ignore]
    pub opencl_program: Option<Program>,
    #[savefile_ignore]
    #[savefile_introspect_ignore]
    pub opencl_propagate_kernel: Option<Kernel>,
    #[savefile_ignore]
    #[savefile_introspect_ignore]
    pub opencl_back_propagate_kernel: Option<Kernel>,
}

impl<'a> TanH<'a> {
    pub fn new(inputs_amount: usize) -> TanH<'a> {
        TanH {
            inputs_amount,
            opencl_context: None,
            opencl_queue: None,
            opencl_program: None,
            opencl_propagate_kernel: None,
            opencl_back_propagate_kernel: None,
            last_outputs_buffer: None,
            last_inputs_buffer: None,
        }
    }
}

impl<'a> Layer<'a> for TanH<'a> {
    fn init(
        &mut self,
        queue: &'a CommandQueue,
        context: &'a Context,
    ) -> Result<(), CompilationOrOpenCLError> {
        let program =
            Program::create_and_build_from_source(context, PROGRAM_SOURCE, "")?;

        let propagation_kernel = Kernel::create(&program, PROPAGATE_KERNEL_NAME)?;
        let back_propagation_kernel = Kernel::create(&program, BACK_PROPAGATE_KERNEL_NAME)?;

        self.opencl_program = Some(program);
        self.opencl_propagate_kernel = Some(propagation_kernel);
        self.opencl_back_propagate_kernel = Some(back_propagation_kernel);
        self.opencl_queue = Some(queue);
        self.opencl_context = Some(context);

        Ok(())
    }

    fn get_last_inputs(&self) -> Option<&Buffer<cl_float>> {
        self.last_inputs_buffer.as_ref()
    }

    fn get_last_outputs(&self) -> Option<&Buffer<cl_float>> {
        self.last_outputs_buffer.as_ref()
    }

    fn get_inputs_amount(&self) -> usize {
        self.inputs_amount
    }

    fn get_outputs_amount(&self) -> usize {
        self.inputs_amount
    }

    fn clean_up_gpu_state(&mut self) -> () {
        if self.last_inputs_buffer.is_some() {
            drop(self.last_inputs_buffer.as_ref().unwrap());
        }

        if self.last_outputs_buffer.is_some() {
            drop(self.last_outputs_buffer.as_ref().unwrap());
        }
    }

    fn sync_data_from_gpu_with_cpu(&mut self) -> Result<(), ClError> {
        Ok(())
    }

    fn propagate(&mut self, inputs: &Buffer<cl_float>) -> Result<&Buffer<cl_float>, ClError> {
        assert!(self.opencl_context.is_some());
        assert!(self.opencl_queue.is_some());

        let context = self.opencl_context.unwrap();
        let queue = self.opencl_queue.unwrap();

        let inputs_size = inputs.size()?;
        let inputs_total_count = inputs_size / mem::size_of::<cl_float>();

        let mut copied_last_inputs_buffer = Buffer::<cl_float>::create(
            context,
            CL_MEM_READ_ONLY,
            inputs_total_count,
            ptr::null_mut(),
        )?;

        // TODO: make copying this into the last inputs optional since this is only needed
        // for fitting a model as to make everything more optimized both in RAM usage and computation
        queue
            .enqueue_copy_buffer(
                inputs,
                &mut copied_last_inputs_buffer,
                0,
                0,
                inputs_size,
                &[],
            )?
            .wait()?;

        self.last_inputs_buffer = Some(copied_last_inputs_buffer);

        let outputs_total_count = inputs.size()? / mem::size_of::<cl_float>();

        let outputs_buffer = Buffer::<cl_float>::create(
            self.opencl_context.unwrap(),
            CL_MEM_READ_WRITE,
            outputs_total_count,
            ptr::null_mut(),
        )?;

        ExecuteKernel::new(self.opencl_propagate_kernel.as_ref().unwrap())
            .set_arg(inputs)
            .set_arg(&outputs_buffer)
            .set_arg(&(outputs_total_count as cl_int))
            .set_global_work_size(outputs_total_count)
            .enqueue_nd_range(self.opencl_queue.unwrap())?
            .wait()?;

        self.last_outputs_buffer = Some(outputs_buffer);

        Ok(self.last_outputs_buffer.as_ref().unwrap())
    }

    fn back_propagate(
        &mut self,
        should_calculate_input_to_error_derivative: bool,
        layer_output_to_error_derivative: &Buffer<cl_float>,
        _: cl_float,
    ) -> Result<Option<Buffer<cl_float>>, ClError> {
        if should_calculate_input_to_error_derivative {
            assert!(self.opencl_context.is_some());
            assert!(self.opencl_queue.is_some());

            let samples_amount = self.last_outputs_buffer.as_ref().unwrap().size()?
                / self.inputs_amount
                / mem::size_of::<cl_float>();

            assert_eq!(samples_amount % 1, 0);

            let loss_to_input_derivatives_buffer = Buffer::<cl_float>::create(
                self.opencl_context.unwrap(),
                CL_MEM_READ_WRITE,
                self.inputs_amount * samples_amount,
                ptr::null_mut(),
            )?;

            ExecuteKernel::new(self.opencl_back_propagate_kernel.as_ref().unwrap())
                .set_arg(layer_output_to_error_derivative)
                .set_arg(self.last_outputs_buffer.as_ref().unwrap())
                .set_arg(&loss_to_input_derivatives_buffer)
                .set_arg(&(self.inputs_amount as cl_int))
                .set_arg(&(samples_amount as cl_int))
                .set_arg(&(self.inputs_amount as cl_int))
                .set_global_work_sizes(&[samples_amount, self.inputs_amount])
                .enqueue_nd_range(self.opencl_queue.unwrap())?
                .wait()?;

            Ok(Some(loss_to_input_derivatives_buffer))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tanh_tests {
    use std::ptr;

    use opencl3::{
        command_queue::{CommandQueue, CL_BLOCKING, CL_NON_BLOCKING},
        context::Context,
        device::{cl_float, get_all_devices, Device, CL_DEVICE_TYPE_CPU},
        memory::{Buffer, CL_MEM_READ_ONLY},
    };
    use rand::{thread_rng, Rng};

    use crate::{types::CompilationOrOpenCLError, utils::approx_eq::assert_approx_equal_distance, layers::Layer};

    use super::TanH;

    #[test]
    fn should_return_same_value_as_normal_tanh_function() -> Result<(), CompilationOrOpenCLError> {
        let device_ids = get_all_devices(CL_DEVICE_TYPE_CPU)?;
        let first_device = Device::new(device_ids[0]);

        let context = Context::from_device(&first_device)?;
        let queue = CommandQueue::create_with_properties(&context, device_ids[0], 0, 0)?;

        let samples_amount = 121;
        let numbers_amount = 13;

        let mut tanh = TanH::new(numbers_amount);
        tanh.init(&queue, &context)?;

        let mut rng = thread_rng();
        let input_samples: Vec<f32> = (0..(samples_amount * numbers_amount))
            .into_iter()
            .map(|_| rng.gen_range(-1.0_f32..1.0_f32))
            .collect();
        let expected_outputs: Vec<f32> = input_samples.iter().map(|x| x.tanh()).collect();

        let mut input_samples_buffer = Buffer::<cl_float>::create(
            &context,
            CL_MEM_READ_ONLY,
            numbers_amount * samples_amount,
            ptr::null_mut(),
        )?;

        queue
            .enqueue_write_buffer(
                &mut input_samples_buffer,
                CL_BLOCKING,
                0,
                input_samples.as_slice(),
                &[],
            )?
            .wait()?;

        let actual_outputs_buffer = tanh.propagate(&input_samples_buffer)?;

        let mut actual_outputs = vec![0.0; numbers_amount * samples_amount];
        let actual_outputs_slice = actual_outputs.as_mut_slice();
        queue
            .enqueue_read_buffer(
                &actual_outputs_buffer,
                CL_BLOCKING,
                0,
                actual_outputs_slice,
                &[],
            )?
            .wait()?;

        assert_approx_equal_distance(
            &expected_outputs,
            &actual_outputs,
            0.01,
        );

        Ok(())
    }

    #[test]
    fn should_back_propagate_returning_the_correct_derivatives() -> Result<(), CompilationOrOpenCLError>
    {
        let device_ids = get_all_devices(CL_DEVICE_TYPE_CPU)?;
        let first_device = Device::new(device_ids[0]);

        let context = Context::from_device(&first_device)?;
        let queue = CommandQueue::create_with_properties(&context, device_ids[0], 0, 0)?;

        let samples_amount = 135;
        let numbers_amount = 19;

        let mut tanh = TanH::new(numbers_amount);
        tanh.init(&queue, &context)?;

        let mut rng = thread_rng();
        let input_samples: Vec<f32> = (0..(samples_amount * numbers_amount))
            .into_iter()
            .map(|_| rng.gen_range(-1.0_f32..1.0_f32))
            .collect();
        let first_derivatives: Vec<f32> = (0..(samples_amount * numbers_amount))
            .into_iter()
            .map(|_| {
                rng.gen_range(-1.0_f32..1.0_f32)
            })
            .collect();

        let mut input_samples_buffer = Buffer::<cl_float>::create(
            &context,
            CL_MEM_READ_ONLY,
            numbers_amount * samples_amount,
            ptr::null_mut(),
        )?;
        let mut first_derivatives_buffer = Buffer::<cl_float>::create(
            &context,
            CL_MEM_READ_ONLY,
            numbers_amount * samples_amount,
            ptr::null_mut(),
        )?;

        queue
            .enqueue_write_buffer(
                &mut first_derivatives_buffer,
                CL_BLOCKING,
                0,
                first_derivatives.as_slice(),
                &[],
            )?
            .wait()?;

        queue
            .enqueue_write_buffer(
                &mut input_samples_buffer,
                CL_BLOCKING,
                0,
                input_samples.as_slice(),
                &[],
            )?
            .wait()?;

        tanh.propagate(&input_samples_buffer)?;

        let expected_loss_to_input_derivatives: Vec<f32> = first_derivatives
            .iter()
            .map(|x| 1.0 - x.tanh().powf(2.0))
            .collect();

        let actual_loss_to_input_derivatives_buffer = tanh
            .back_propagate(true, &first_derivatives_buffer, 0.0)?
            .unwrap();
        let mut actual_loss_to_input_derivatives = vec![0.0; numbers_amount * samples_amount];
        let actual_loss_to_input_derivatives_slice =
            actual_loss_to_input_derivatives.as_mut_slice();
        queue
            .enqueue_read_buffer(
                &actual_loss_to_input_derivatives_buffer,
                CL_NON_BLOCKING,
                0,
                actual_loss_to_input_derivatives_slice,
                &[],
            )?
            .wait()?;

        println!(
            "derivatives CPU: {:?}",
            &expected_loss_to_input_derivatives,
        );
        println!("\nderivatives GPU: {:?}", &actual_loss_to_input_derivatives);

        assert_approx_equal_distance(
            &actual_loss_to_input_derivatives,
            &expected_loss_to_input_derivatives,
            0.01,
        );

        Ok(())
    }
}