//! Feed-forward neural network (`docs/spec/brain.md`, "NeuralNetwork.swift").
//!
//! Dense layers with He-initialised weights, ReLU hidden units, and an
//! explicit output activation. Training is one SGD step per call with
//! per-unit gradient clipping at ±1.0 — unchanged from the original design.
//!
//! Randomness is injected through [`Rng`], a tiny deterministic generator, so
//! initialisation is reproducible from a seed and no `rand` dependency is
//! needed.

/// A small deterministic PRNG (splitmix64) used for weight initialisation,
/// replay sampling, and ε-greedy exploration.
///
/// Seeded from a `u64`, so every consumer is reproducible in tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rng(u64);

impl Rng {
    /// Creates a generator from a seed. Any seed, including zero, is fine.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// Next raw 64-bit value (splitmix64).
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform `f32` in `[0, 1)`, built from the top 24 bits so the value is
    /// exactly representable.
    pub fn next_f32(&mut self) -> f32 {
        // 24 bits fit an f32 mantissa exactly, so the cast is lossless.
        let mantissa = (self.next_u64() >> 40) as f32;
        mantissa / 16_777_216.0
    }

    /// Uniform `f32` in `[lo, hi)`.
    pub fn next_f32_in(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next_f32()
    }

    /// Uniform `usize` in `[0, n)`.
    ///
    /// # Panics
    /// Panics if `n == 0` — there is no value to draw.
    pub fn next_usize_below(&mut self, n: usize) -> usize {
        assert!(n > 0, "next_usize_below needs a non-empty range");
        // The remainder is below `n`, which already fits in usize.
        (self.next_u64() % n as u64) as usize
    }
}

/// Activation applied to the output layer only. Hidden layers are always ReLU.
///
/// Open decision carried over from the spec: the original design softmaxed
/// the output even when the net served as a Q-function, which destroys the
/// magnitude and sign a Bellman update depends on. The choice is therefore
/// explicit here rather than silently changed: [`Linear`](Self::Linear) is
/// correct for DQN (and is what [`crate::brain::dqn::DqnAgent`] uses);
/// [`Softmax`](Self::Softmax) preserves the old behaviour for a classifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputActivation {
    /// Identity — outputs are raw Q-values / regression targets.
    Linear,
    /// Max-subtracted softmax — outputs form a probability simplex. Kept
    /// under `cfg(test)`: nothing in the bot constructs a classifier, but the
    /// spec's open decision is preserved and exercised by the tests.
    #[cfg(test)]
    Softmax,
}

/// One fully-connected layer: `output = weights · input + biases`.
#[derive(Debug, Clone, PartialEq)]
pub struct DenseLayer {
    /// Row-major `[output_count * input_count]`.
    pub weights: Vec<f32>,
    /// `[output_count]`.
    pub biases: Vec<f32>,
    pub input_count: usize,
    pub output_count: usize,
}

impl DenseLayer {
    /// He initialisation — the correct pairing for ReLU hidden layers.
    /// Weights are uniform in `±sqrt(2 / input_count)`, biases zero.
    #[must_use]
    pub fn new(input_count: usize, output_count: usize, rng: &mut Rng) -> Self {
        assert!(input_count > 0, "a layer needs at least one input");
        let scale = (2.0 / input_count as f32).sqrt();
        let weights = (0..input_count * output_count)
            .map(|_| rng.next_f32_in(-scale, scale))
            .collect();
        Self {
            weights,
            biases: vec![0.0; output_count],
            input_count,
            output_count,
        }
    }

    /// Dot product of `weights[row]` and `input`.
    ///
    /// The Swift original accumulates in SIMD8 lanes; a plain loop is kept
    /// here for clarity — the compiler vectorises it, and the result is the
    /// same up to floating-point association.
    #[must_use]
    pub fn dot(&self, row: usize, input: &[f32]) -> f32 {
        debug_assert_eq!(input.len(), self.input_count);
        let base = row * self.input_count;
        self.weights[base..base + self.input_count]
            .iter()
            .zip(input)
            .map(|(w, x)| w * x)
            .sum()
    }
}

/// A feed-forward network described by its `topology` (layer widths).
#[derive(Debug, Clone, PartialEq)]
pub struct NeuralNetwork {
    pub layers: Vec<DenseLayer>,
    /// e.g. `[128, 64, 32, 3]`.
    pub topology: Vec<usize>,
    pub output_activation: OutputActivation,
}

impl NeuralNetwork {
    /// Builds a network with one [`DenseLayer`] per consecutive pair in
    /// `topology`, drawing initial weights from `rng`.
    ///
    /// # Panics
    /// Panics if `topology` has fewer than two entries or contains a zero
    /// width.
    #[must_use]
    pub fn new(topology: &[usize], output_activation: OutputActivation, rng: &mut Rng) -> Self {
        assert!(
            topology.len() >= 2,
            "need at least an input and an output layer"
        );
        assert!(
            topology.iter().all(|&w| w > 0),
            "every layer needs at least one unit"
        );
        let layers = topology
            .windows(2)
            .map(|pair| DenseLayer::new(pair[0], pair[1], rng))
            .collect();
        Self {
            layers,
            topology: topology.to_vec(),
            output_activation,
        }
    }

    /// Width of the input layer.
    #[must_use]
    pub fn input_count(&self) -> usize {
        self.topology[0]
    }

    /// Width of the output layer.
    #[must_use]
    pub fn output_count(&self) -> usize {
        self.topology[self.topology.len() - 1]
    }

    /// Inference only — non-mutating so the target network can be read freely.
    #[must_use]
    pub fn forward(&self, input: &[f32]) -> Vec<f32> {
        self.forward_retaining_activations(input).0
    }

    /// Returns `(output, pre, post)`: every layer's pre-activation (`z`) and
    /// post-activation values — backprop needs both. `post[0]` is the input.
    ///
    /// # Panics
    /// Panics if `input` is not `topology[0]` wide.
    #[must_use]
    pub fn forward_retaining_activations(
        &self,
        input: &[f32],
    ) -> (Vec<f32>, Vec<Vec<f32>>, Vec<Vec<f32>>) {
        assert_eq!(
            input.len(),
            self.input_count(),
            "input width must match the input layer"
        );
        let mut pre: Vec<Vec<f32>> = Vec::with_capacity(self.layers.len());
        let mut post: Vec<Vec<f32>> = Vec::with_capacity(self.layers.len() + 1);
        post.push(input.to_vec());
        let mut activation = input.to_vec();

        let last = self.layers.len() - 1;
        for (idx, layer) in self.layers.iter().enumerate() {
            let z: Vec<f32> = (0..layer.output_count)
                .map(|row| layer.dot(row, &activation) + layer.biases[row])
                .collect();
            activation = if idx == last {
                self.apply_output_activation(&z)
            } else {
                z.iter().copied().map(Self::relu).collect()
            };
            pre.push(z);
            post.push(activation.clone());
        }
        (activation, pre, post)
    }

    fn apply_output_activation(&self, z: &[f32]) -> Vec<f32> {
        match self.output_activation {
            OutputActivation::Linear => z.to_vec(),
            #[cfg(test)]
            OutputActivation::Softmax => Self::softmax(z),
        }
    }

    /// One SGD step. Per-unit gradients are clipped at ±1.0 (unchanged from
    /// the original design).
    ///
    /// `target` is a full output-width vector; for DQN only the taken
    /// action's slot differs from the current prediction, so the other slots
    /// contribute zero error and the update is effectively single-action.
    ///
    /// # Panics
    /// Panics if `target` is not `output_count` wide.
    pub fn train(&mut self, input: &[f32], target: &[f32], lr: f32) {
        let (output, pre, post) = self.forward_retaining_activations(input);
        assert_eq!(
            target.len(),
            output.len(),
            "target width must match output width"
        );

        // Output-layer error. For Linear + squared error, and for Softmax +
        // cross-entropy, dL/dz reduces to the same (output - target) — the
        // activation derivative cancels in both pairings.
        let mut delta: Vec<f32> = output.iter().zip(target).map(|(o, t)| o - t).collect();

        for idx in (0..self.layers.len()).rev() {
            let layer = &mut self.layers[idx];
            let input_to_layer = &post[idx];
            let mut next_delta = vec![0.0f32; layer.input_count];

            let rows = layer.weights.chunks_exact_mut(layer.input_count);
            for ((row_weights, bias), &raw_delta) in rows.zip(&mut layer.biases).zip(&delta) {
                let d = Self::clip(raw_delta);
                if d == 0.0 {
                    continue;
                }
                for ((w, nd), &x) in row_weights
                    .iter_mut()
                    .zip(&mut next_delta)
                    .zip(input_to_layer)
                {
                    // Propagate before the weight is overwritten.
                    *nd += *w * d;
                    *w -= lr * d * x;
                }
                *bias -= lr * d;
            }

            if idx > 0 {
                // ReLU derivative: pass the gradient only where the unit fired.
                let z = &pre[idx - 1];
                for (nd, &zv) in next_delta.iter_mut().zip(z) {
                    if zv <= 0.0 {
                        *nd = 0.0;
                    }
                }
            }
            delta = next_delta;
        }
    }

    /// Rectified linear unit.
    #[must_use]
    pub fn relu(x: f32) -> f32 {
        x.max(0.0)
    }

    /// Gradient clip to `[-1, 1]`.
    #[must_use]
    pub fn clip(g: f32) -> f32 {
        g.clamp(-1.0, 1.0)
    }

    /// Max-subtracted softmax for numerical stability — raw `exp` overflows
    /// on large logits. Empty input, or an input whose exponentials sum to
    /// zero, is returned unchanged.
    #[cfg(test)]
    #[must_use]
    pub fn softmax(z: &[f32]) -> Vec<f32> {
        let Some(max_z) = z.iter().copied().reduce(f32::max) else {
            return z.to_vec();
        };
        let exps: Vec<f32> = z.iter().map(|&v| (v - max_z).exp()).collect();
        let sum: f32 = exps.iter().sum();
        if sum > 0.0 {
            exps.iter().map(|e| e / sum).collect()
        } else {
            z.to_vec()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_per_seed() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        let xs: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        let ys: Vec<u64> = (0..8).map(|_| b.next_u64()).collect();
        assert_eq!(xs, ys);
        let mut c = Rng::new(43);
        assert_ne!(xs[0], c.next_u64());
    }

    #[test]
    fn rng_ranges_are_honoured() {
        let mut rng = Rng::new(7);
        for _ in 0..10_000 {
            let f = rng.next_f32_in(-0.5, 0.5);
            assert!((-0.5..0.5).contains(&f), "{f} out of range");
            let u = rng.next_usize_below(3);
            assert!(u < 3);
        }
    }

    #[test]
    fn he_init_is_bounded_and_biases_zero() {
        let mut rng = Rng::new(1);
        let layer = DenseLayer::new(8, 4, &mut rng);
        let scale = (2.0f32 / 8.0).sqrt();
        assert_eq!(layer.weights.len(), 32);
        assert!(layer.weights.iter().all(|w| w.abs() <= scale));
        assert_eq!(layer.biases, vec![0.0; 4]);
    }

    #[test]
    fn dot_matches_manual_sum() {
        let layer = DenseLayer {
            weights: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            biases: vec![0.0, 0.0],
            input_count: 3,
            output_count: 2,
        };
        let input = [1.0, 0.5, -1.0];
        assert!((layer.dot(0, &input) - (1.0 + 1.0 - 3.0)).abs() < 1e-6);
        assert!((layer.dot(1, &input) - (4.0 + 2.5 - 6.0)).abs() < 1e-6);
    }

    #[test]
    fn forward_output_width_equals_last_topology_entry() {
        let mut rng = Rng::new(3);
        let net = NeuralNetwork::new(&[128, 64, 32, 3], OutputActivation::Linear, &mut rng);
        let out = net.forward(&vec![0.1; 128]);
        assert_eq!(out.len(), 3);
        assert_eq!(net.layers.len(), 3);
        let (o, pre, post) = net.forward_retaining_activations(&vec![0.1; 128]);
        assert_eq!(o, out);
        assert_eq!(pre.len(), 3);
        assert_eq!(post.len(), 4);
        assert_eq!(post[0].len(), 128);
    }

    #[test]
    fn hidden_activations_are_non_negative() {
        let mut rng = Rng::new(5);
        let net = NeuralNetwork::new(&[4, 6, 2], OutputActivation::Linear, &mut rng);
        let (_, _, post) = net.forward_retaining_activations(&[1.0, -1.0, 2.0, -2.0]);
        assert!(post[1].iter().all(|&v| v >= 0.0));
    }

    #[test]
    fn softmax_sums_to_one_and_is_stable_on_large_logits() {
        let p = NeuralNetwork::softmax(&[1000.0, 1000.0, 0.0]);
        let sum: f32 = p.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5, "sum was {sum}");
        assert!(p.iter().all(|v| v.is_finite()));
        assert!((p[0] - 0.5).abs() < 1e-5);
        assert!(p[2] < 1e-6);

        let q = NeuralNetwork::softmax(&[1.0, 2.0, 3.0]);
        let sum: f32 = q.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
        assert!(q[2] > q[1] && q[1] > q[0]);

        assert!(NeuralNetwork::softmax(&[]).is_empty());
    }

    #[test]
    fn softmax_output_activation_yields_simplex() {
        let mut rng = Rng::new(9);
        let net = NeuralNetwork::new(&[3, 5, 4], OutputActivation::Softmax, &mut rng);
        let out = net.forward(&[0.3, -0.7, 1.2]);
        let sum: f32 = out.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
        assert!(out.iter().all(|&v| v >= 0.0));
    }

    #[test]
    fn relu_and_clip() {
        assert_eq!(NeuralNetwork::relu(-2.0), 0.0);
        assert_eq!(NeuralNetwork::relu(2.0), 2.0);
        assert_eq!(NeuralNetwork::clip(5.0), 1.0);
        assert_eq!(NeuralNetwork::clip(-5.0), -1.0);
        assert_eq!(NeuralNetwork::clip(0.25), 0.25);
    }

    fn mse(net: &NeuralNetwork, samples: &[(f32, f32)]) -> f32 {
        samples
            .iter()
            .map(|&(x, y)| {
                let out = net.forward(&[x])[0];
                (out - y).powi(2)
            })
            .sum::<f32>()
            / samples.len() as f32
    }

    #[test]
    fn train_learns_a_tiny_regression() {
        let mut rng = Rng::new(11);
        let mut net = NeuralNetwork::new(&[1, 8, 1], OutputActivation::Linear, &mut rng);
        let samples: Vec<(f32, f32)> = (0..20)
            .map(|i| i as f32 / 20.0)
            .map(|x| (x, 2.0 * x))
            .collect();
        let before = mse(&net, &samples);
        for _ in 0..500 {
            for &(x, y) in &samples {
                net.train(&[x], &[y], 0.01);
            }
        }
        let after = mse(&net, &samples);
        assert!(after < before, "loss did not drop: {before} -> {after}");
        assert!(after < 0.01, "loss still high: {after}");
    }

    #[test]
    #[should_panic(expected = "target width must match output width")]
    fn train_panics_on_target_width_mismatch() {
        let mut rng = Rng::new(2);
        let mut net = NeuralNetwork::new(&[2, 3, 2], OutputActivation::Linear, &mut rng);
        net.train(&[0.0, 1.0], &[1.0], 0.01);
    }

    #[test]
    #[should_panic(expected = "need at least an input and an output layer")]
    fn new_rejects_single_layer_topology() {
        let mut rng = Rng::new(2);
        let _ = NeuralNetwork::new(&[4], OutputActivation::Linear, &mut rng);
    }
}
