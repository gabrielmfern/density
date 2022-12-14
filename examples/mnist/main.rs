use intricate::{
    datasets::mnist,
    layers::{
        activations::{SoftMax, TanH, ReLU},
        Dense, Conv2D,
    },
    loss_functions::CategoricalCrossEntropy,
    optimizers::{NesterovMomentumAcceleratedOptimizer, BasicOptimizer, AdagradOptimizer},
    types::{TrainingOptions, TrainingVerbosity},
    utils::{opencl::DeviceType, setup_opencl},
    Model,
};
use savefile::save_file;

const MODEL_PATH: &str = "mnist-model.bin";

fn main() -> () {
    // don't really recommend using CPU for this, but it is possible as long as you have drivers
    let state = setup_opencl(DeviceType::GPU).expect("unable to setup OpenCL");

    let training_inputs = mnist::get_training_inputs();
    let training_outputs = mnist::get_training_outputs();

    let mut mnist_model: Model = Model::new(vec![
        Conv2D::new((28, 28), (4, 4)),
        ReLU::new(25 * 25),

        Conv2D::new((25, 25), (4, 4)),
        ReLU::new(22 * 22),

        Dense::new(22 * 22, 10),
        SoftMax::new(10),
    ]);

    mnist_model
        .init(&state)
        .expect("unable to initialize Mnist model");

    let mut loss_fn = CategoricalCrossEntropy::new();
    let mut optimizer = BasicOptimizer::new(0.01);

    mnist_model
        .fit(
            &training_inputs,
            &training_outputs,
            &mut TrainingOptions {
                loss_fn: &mut loss_fn,
                optimizer: &mut optimizer,
                batch_size: 256, // try increasing this based on how much your GPU can take
                                 // on by batch
                verbosity: TrainingVerbosity {
                    show_current_epoch: true,
                    show_epoch_progress: true,
                    show_epoch_elapsed: true,
                    print_loss: true,
                    print_accuracy: true,
                    halting_condition_warning: false,
                },
                halting_condition: None,
                compute_loss: true,
                compute_accuracy: true,
                epochs: 100,
            },
        )
        .expect("unable to fit Mnist model");

    mnist_model
        .sync_data_from_buffers_to_host()
        .expect("unable to sync weights from the GPU");

    save_file(MODEL_PATH, 0, &mnist_model).expect("unable to save Mnist model");
}