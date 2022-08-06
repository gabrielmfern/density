#[allow(unused_imports)]
use crate::layers::Layer;
use crate::layers::OpenCLLayer;
#[allow(unused_imports)]
use crate::utils::approx_eq::assert_approx_equal_distance;

use std::mem;
use std::ptr;
use opencl3::memory::ClMem;
#[allow(unused_imports)]
use opencl3::{
    command_queue::{CommandQueue, CL_BLOCKING, CL_NON_BLOCKING},
    context::Context,
    device::{cl_float, get_all_devices, Device, CL_DEVICE_TYPE_GPU},
    error_codes::{cl_int, ClError},
    kernel::{ExecuteKernel, Kernel},
    memory::{Buffer, CL_MEM_READ_ONLY, CL_MEM_READ_WRITE},
    program::Program,
};

use savefile_derive::Savefile;

#[allow(unused_imports)]
use super::tanh::TanH;

const PROGRAM_SOURCE: &str = include_str!("kernels/tanh.cl");
const PROPAGATE_KERNEL_NAME: &str = "propagate";
const BACK_PROPAGATE_KERNEL_NAME: &str = "back_propagate";

#[test]
fn should_return_same_value_as_normal_tanh_function() -> Result<(), ClError> {
    let device_ids = get_all_devices(CL_DEVICE_TYPE_GPU)?;
    let first_device = Device::new(device_ids[0]);

    let context = Context::from_device(&first_device)?;
    let queue = CommandQueue::create_with_properties(&context, device_ids[0], 0, 0)?;

    let mut normal_tanh = TanH::new();
    let mut gpu_tanh = TanHGPU::new(&context, &queue)?;
    
    let input_samples = vec![vec![0.412; 100]; 100];
    let expected_outputs = normal_tanh.propagate(&input_samples);

    let mut input_samples_buffer = Buffer::<cl_float>::create(
        &context,
        CL_MEM_READ_ONLY,
        100 * 100,
        ptr::null_mut()
    )?;

    queue.enqueue_write_buffer(
        &mut input_samples_buffer, 
        CL_BLOCKING, 
        0, 
        input_samples.iter().map(|v| v.to_vec()).flatten().collect::<Vec<f32>>().as_slice(),
        &[]
    )?.wait()?;

    let actual_outputs_buffer = gpu_tanh.propagate(&input_samples_buffer)?;

    let mut actual_outputs = vec![0.0; 100 * 100];
    let actual_outputs_slice = actual_outputs.as_mut_slice();
    queue.enqueue_read_buffer(
        &actual_outputs_buffer, 
        CL_BLOCKING,
        0, 
        actual_outputs_slice, 
        &[]
    )?.wait()?;

    assert_approx_equal_distance(&expected_outputs.iter().map(|v| v.to_vec()).flatten().collect(), &actual_outputs, 0.2);

    Ok(())
}

#[derive(Debug, Savefile)]
pub struct TanHGPU<'a> {
    #[savefile_ignore]
    #[savefile_introspect_ignore]
    pub last_inputs_buffer: Option<&'a Buffer<cl_float>>,
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
    pub opencl_back_propagate_kernel: Option<Kernel>
}

impl<'a> TanHGPU<'a> {
    #[allow(dead_code)]
    fn new(context: &'a Context, queue: &'a CommandQueue) -> Result<TanHGPU<'a>, ClError> {
        let program_compilation_result =
            Program::create_and_build_from_source(context, PROGRAM_SOURCE, "");
        if program_compilation_result .is_err() {
            println!(
                "A compilation error was found in the tanh.cl Program:\n{:?}",
                program_compilation_result .err().unwrap()
            );
            println!("Please report this issue at https://github.com/gabrielmfern/intricate");
            panic!();
        }

        let program = program_compilation_result .unwrap();
        let propagation_kernel = Kernel::create(&program, PROPAGATE_KERNEL_NAME)?;
        let back_propagation_kernel = Kernel::create(&program, BACK_PROPAGATE_KERNEL_NAME)?;

        Ok(TanHGPU {
            opencl_context: Some(context),
            opencl_queue: Some(queue),
            opencl_program: Some(program),
            opencl_propagate_kernel: Some(propagation_kernel),
            opencl_back_propagate_kernel: Some(back_propagation_kernel),
            last_outputs_buffer: None,
            last_inputs_buffer: None
        })
    }
}

impl<'a> OpenCLLayer<'a> for TanHGPU<'a> {
    fn send_to_gpu(
        &mut self,
        queue: &'a CommandQueue,
        context: &'a Context,
    ) -> Result<(), ClError> {
        let program_compilation_result =
            Program::create_and_build_from_source(context, PROGRAM_SOURCE, "");
        if program_compilation_result .is_err() {
            println!(
                "A compilation error was found in the tanh.cl Program:\n{:?}",
                program_compilation_result .err().unwrap()
            );
            println!("Please report this issue at https://github.com/gabrielmfern/intricate");
            panic!();
        }

        let program = program_compilation_result.unwrap();
        let propagation_kernel = Kernel::create(&program, PROPAGATE_KERNEL_NAME)?;
        let back_propagation_kernel = Kernel::create(&program, BACK_PROPAGATE_KERNEL_NAME)?;

        self.opencl_program = Some(program);
        self.opencl_propagate_kernel = Some(propagation_kernel);
        self.opencl_back_propagate_kernel = Some(back_propagation_kernel);
        self.opencl_queue = Some(queue);
        self.opencl_context = Some(context);

        Ok(())
    }

    fn get_last_inputs(&self) -> Option<&'a Buffer<cl_float>> {
        self.last_inputs_buffer
    }

    fn get_last_outputs(&self) -> Option<&Buffer<cl_float>> {
        self.last_outputs_buffer.as_ref()
    }

    fn get_inputs_amount(&self) -> usize {
        0
    }

    fn get_outputs_amount(&self) -> usize {
        0
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

    fn propagate(
        &mut self,
        inputs: &'a Buffer<cl_float>,
    ) -> Result<&Buffer<cl_float>, ClError> {
        assert!(self.opencl_context.is_some());
        assert!(self.opencl_queue.is_some());
        if self.last_inputs_buffer.is_some() {
            drop(self.last_inputs_buffer.as_ref().unwrap());
        }
        if self.last_outputs_buffer.is_some() {
            drop(self.last_outputs_buffer.as_ref().unwrap());
        }
        
        self.last_inputs_buffer = Some(inputs);

        let outputs_total_count = inputs.size()? / mem::size_of::<cl_float>();

        let outputs_buffer = Buffer::<cl_float>::create(
            self.opencl_context.unwrap(),
            CL_MEM_READ_WRITE,
            outputs_total_count,
            ptr::null_mut()
        )?;

        ExecuteKernel::new(self.opencl_propagate_kernel.as_ref().unwrap())
            .set_arg(inputs)
            .set_arg(&outputs_buffer)
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

            let outputs_total_count = 
                self.last_outputs_buffer.as_ref().unwrap().size()? / mem::size_of::<cl_float>();

            let loss_to_input_derivatives_buffer = Buffer::<cl_float>::create(
                self.opencl_context.unwrap(),
                CL_MEM_READ_WRITE,
                outputs_total_count,
                ptr::null_mut()
            )?;

            ExecuteKernel::new(self.opencl_back_propagate_kernel.as_ref().unwrap())
                .set_arg(layer_output_to_error_derivative)
                .set_arg(self.last_outputs_buffer.as_ref().unwrap())
                .set_arg(&loss_to_input_derivatives_buffer)
                .set_global_work_size(outputs_total_count)
                .enqueue_nd_range(self.opencl_queue.unwrap())?
                .wait()?;

            Ok(Some(loss_to_input_derivatives_buffer))
        } else {
            Ok(None)
        }
    }
}
