# Intricate

[![Crates.io](https://img.shields.io/crates/v/intricate.svg?label=intricate)](https://crates.io/crates/intricate)

A GPU accelerated library that creates/trains/runs neural networks in pure safe Rust code.

## Architechture overview

Intricate has a layout very similar to popular libraries out there such as Keras.

### Models

As said before, similar to Tensorflow, Intricate defines Models as basically
a list of `Layers` that are explained down bellow.

### Layers

Every layer receives **inputs** and returns **outputs**, 
they must also implement a `back_propagate` method that 
will mutate the layer if needed and then return the derivatives
of the loss function with respected to the inputs, 
written with **I** as the inputs of the layer, 
**E** as the loss and **O** as the outputs of the layer:

```
dE/dI <- Model <- dE/dO
```

These layers can be anything you want and just propagates the previous inputs
to the next inputs for the next layer or for the outputs of the whole Model.

There are a few activations already implemented, but still many to be implemented.

## XoR using Intricate

If you look at the `examples/` in the repository 
you will find XoR implemented using Intricate. The code goes like this:

```rs
// Defining the training data
let training_inputs = Vec::from([
    Vec::from([0.0, 0.0]),
    Vec::from([0.0, 1.0]),
    Vec::from([1.0, 0.0]),
    Vec::from([1.0, 1.0]),
]);
let expected_outputs = Vec::from([
    Vec::from([0.0]),
    Vec::from([1.0]),
    Vec::from([1.0]),
    Vec::from([0.0]),
]);
```

```rs
// Defining the layers for our XoR Model
let mut layers: Vec<Box<dyn Layer<f64>>> = Vec::new();

layers.push(Box::new(DenseF64::new(2, 3)));
// The tanh activation function
layers.push(Box::new(TanHF64::new()));
layers.push(Box::new(DenseF64::new(3, 1)));
layers.push(Box::new(TanHF64::new()));
```

```rs
// Fit the model however many times we want
xor_model.fit(
    &training_inputs, 
    &expected_outputs, 
    TrainingOptionsF64 {
        learning_rate: 0.1,
        loss_algorithm: Box::new(MeanSquared), // The Mean Squared loss function
        should_print_information: true, // Should be verbose
        use_gpu: false // Should initialize WGPU Device and Queue for GPU layers
    },
    10000 // Epochs
).await;
// we await here because for a GPU computation type of layer
// the responses from the GPU must be awaited on the CPU
```

As you can see it is extremely easy creating these models, and blazingly fast as well.

## Things to be done still
- writing some kind of macro to generate the code for f32 and f64 versions of certain structs and traits to not have duplicated code.
- making so that the 'get' methods implemented return slices instead of copies of the vectors as to not duplicate things in RAM and save as much RAM as possible for very large models.
- improve the GPU shaders, perhaps finding a way to send the full unflattened matrices to the GPU instead of sending just a flattened array.
- create GPU accelerated activations and loss functions as to make everything GPU accelerated.
- perhaps write some shader to calculate the Model **loss** to **output** gradient (derivatives).
- implement convolutional layers and perhaps even solve some image classification problems in a example
- add a example that uses GPU acceleration
