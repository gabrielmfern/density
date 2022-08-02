use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use std::borrow::Cow;
use wgpu::util::DeviceExt;

use crate::gpu::{
    make_compute_storage_bind_group_layout_entry, make_compute_uniform_bind_group_layout_entry,
};

use crate::layers::dense_gpu::DenseGpuF32;
#[allow(unused_imports)]
use crate::layers::layer::Layer;

#[allow(dead_code)]
pub async fn calculate_dense_input_to_error_derivatives(
    dense: &mut DenseGpuF32,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layer_output_to_error_derivatives: &Vec<Vec<f32>>,
) -> Option<Vec<Vec<f32>>> {
    let flattened_layer_output_to_error_derivatives = layer_output_to_error_derivatives
        .par_iter()
        .map(|x| x.to_vec())
        .flatten()
        .map(|x| x as f32)
        .collect::<Vec<f32>>();

    let flattened_layer_weights = dense
        .weights
        .par_iter()
        .map(|x| x.to_vec())
        .flatten()
        .map(|x| x as f32)
        .collect::<Vec<f32>>();

    let samples_amount = dense.last_inputs.len();

    execute_gpu_code(
        &device,
        &queue,
        samples_amount,
        dense.outputs_amount,
        dense.inputs_amount,
        flattened_layer_output_to_error_derivatives.as_slice(),
        flattened_layer_weights.as_slice(),
    )
    .await
}

async fn execute_gpu_code(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    samples_amount: usize,
    outputs_amount: usize,
    inputs_amount: usize,
    flattened_layer_output_to_error_derivatives: &[f32],
    flattened_layer_weights: &[f32],
) -> Option<Vec<Vec<f32>>> {
    let cs_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!(
            "shaders/calculate_dense_input_to_error_derivatives.wgsl"
        ))),
    });

    let input_derivatives_size = inputs_amount * samples_amount * std::mem::size_of::<f32>();
    let buffer_size = input_derivatives_size as wgpu::BufferAddress;

    let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: buffer_size,
        usage: wgpu::BufferUsages::MAP_WRITE
            | wgpu::BufferUsages::MAP_READ
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    let flattened_layer_weights_buffer =
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("flattened_layer_weights"),
            contents: bytemuck::cast_slice(&flattened_layer_weights),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
        });

    let flattened_layer_output_to_error_derivatives_buffer =
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("flattened_layer_output_to_error_derivatives"),
            contents: bytemuck::cast_slice(&flattened_layer_output_to_error_derivatives),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
        });

    let flattened_input_to_error_derivatives_vec = vec![0.0; inputs_amount * samples_amount];
    let flattened_input_to_error_derivatives = flattened_input_to_error_derivatives_vec.as_slice();
    let flattened_input_to_error_derivatives_buffer =
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("flattened_layer_inputs"),
            contents: bytemuck::cast_slice(&flattened_input_to_error_derivatives),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
        });

    let inputs_amount_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("inputs_amount"),
        contents: &(inputs_amount as u32).to_ne_bytes(),
        usage: wgpu::BufferUsages::UNIFORM
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
    });

    let outputs_amount_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("outputs_amount"),
        contents: &(outputs_amount as u32).to_ne_bytes(),
        usage: wgpu::BufferUsages::UNIFORM
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
    });

    let samples_amount_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("samples_amount"),
        contents: &(samples_amount as u32).to_ne_bytes(),
        usage: wgpu::BufferUsages::UNIFORM
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
    });

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Apply Gradients to Dense Weights Pipeline Bind Group Layout"),
        entries: &[
            make_compute_storage_bind_group_layout_entry(0, false),
            make_compute_storage_bind_group_layout_entry(1, true),
            make_compute_storage_bind_group_layout_entry(2, true),
            make_compute_uniform_bind_group_layout_entry(3),
            make_compute_uniform_bind_group_layout_entry(4),
            make_compute_uniform_bind_group_layout_entry(5),
        ],
    });

    let compute_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("calculate input to error derivatives gpu compute pipeline"),
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });

    let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: Some(&compute_layout),
        module: &cs_module,
        entry_point: "main",
    });

    let bind_group_layout = compute_pipeline.get_bind_group_layout(0);
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: flattened_input_to_error_derivatives_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: flattened_layer_output_to_error_derivatives_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: flattened_layer_weights_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: samples_amount_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: outputs_amount_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: inputs_amount_buffer.as_entire_binding(),
            },
        ],
    });

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None });
        cpass.set_pipeline(&compute_pipeline);
        cpass.set_bind_group(0, &bind_group, &[]);
        cpass.insert_debug_marker("compute input to error derivatives iteration");
        cpass.dispatch_workgroups(
            samples_amount as u32,
            inputs_amount as u32,
            1,
        );
        // Number of cells to run, the (x,y,z) size of item being processed
    }
    encoder.copy_buffer_to_buffer(
        &flattened_input_to_error_derivatives_buffer,
        0,
        &staging_buffer,
        0,
        buffer_size,
    );

    queue.submit(Some(encoder.finish()));

    let buffer_slice = staging_buffer.slice(..);
    let (sender, receiver) = futures_intrusive::channel::shared::oneshot_channel();
    buffer_slice.map_async(wgpu::MapMode::Read, move |v| sender.send(v).unwrap());

    device.poll(wgpu::Maintain::Wait);

    if let Some(Ok(())) = receiver.receive().await {
        let data = buffer_slice.get_mapped_range();
        let result: &[f32] = bytemuck::cast_slice(&data);

        let flattened_actual_input_to_error_derivatives: Vec<f32> = result.to_vec();

        drop(data);

        let actual_input_to_error_derivatives: Vec<Vec<f32>> = (0..samples_amount)
            .into_par_iter()
            .map(|sample_index| {
                let row_part = sample_index * inputs_amount;
                (0..inputs_amount)
                    .into_iter()
                    .map(|input_index| {
                        flattened_actual_input_to_error_derivatives[row_part + input_index] as f32
                    })
                    .collect()
            })
            .collect();

        staging_buffer.unmap();
        Some(actual_input_to_error_derivatives)
    } else {
        panic!("failed to run compute input to error derivatives on gpu!")
    }
}