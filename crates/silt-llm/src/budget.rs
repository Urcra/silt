//! How much memory this machine has for a model: what `llama-server
//! --list-devices` reports, else the machine's memory for a CPU-only build.

use serde::{Deserialize, Serialize};

const MIB: u64 = 1 << 20;
/// Weights and context beyond the model's file: compute buffers and the
/// like. Rough, which is why a near miss is shown as tight.
const OVERHEAD: u64 = 1 << 30;

/// A device `llama-server` can put a model on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    /// Its backend's name for it, such as `CUDA0`, `Vulkan0` or `Metal`.
    pub name: String,
    pub description: String,
    pub total: u64,
    pub free: u64,
}

/// Reads `--list-devices`' lines, `  CUDA0: NVIDIA … (24463 MiB, 23000 MiB free)`;
/// any other line is skipped.
pub fn parse_devices(output: &str) -> Vec<Device> {
    output
        .lines()
        .filter_map(|line| {
            let (name, rest) = line.trim().split_once(": ")?;
            let rest = rest.strip_suffix(')')?;
            let (description, sizes) = rest.rsplit_once(" (")?;
            let (total, free) = sizes.split_once(", ")?;
            let mib = |text: &str| text.trim().strip_suffix(" MiB")?.trim().parse::<u64>().ok().map(|size| size * MIB);
            let free = free.strip_suffix(" free")?;
            if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') {
                return None;
            }
            Some(Device {
                name: name.to_owned(),
                description: description.trim().to_owned(),
                total: mib(total)?,
                free: mib(free)?,
            })
        })
        .collect()
}

/// Memory a model may use here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    pub devices: Vec<Device>,
    /// The machine's memory, which a CPU-only build uses.
    pub memory: u64,
}

impl Budget {
    /// All devices together, as `llama-server` splits a model across them;
    /// the machine's memory when there are none.
    pub fn room(&self) -> u64 {
        if self.devices.is_empty() { self.memory } else { self.devices.iter().map(|device| device.total).sum() }
    }
}

/// How a model sits in a budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    Fits,
    /// Within the last tenth: it may load, with less context or nothing else
    /// running.
    Tight,
    TooBig,
}

/// The memory a model of `weights` bytes needs with `context` tokens, given
/// the KV cache's bytes per token where they're known.
pub fn need(weights: u64, kv_per_token: Option<u64>, context: u32) -> u64 {
    let kv = kv_per_token.map_or(weights / 5, |per_token| per_token.saturating_mul(u64::from(context)));
    weights.saturating_add(kv).saturating_add(OVERHEAD)
}

/// The least and most context Silt starts a model with.
const LEAST_CONTEXT: u32 = 8192;
const MOST_CONTEXT: u32 = 131_072;
/// The context a model starts with when its KV cache's size isn't known yet.
const UNSIZED_CONTEXT: u32 = 32_768;

/// The context to run a model with: the most, as a power of two, that the
/// model was trained for and that fits here with its weights, else the least.
pub fn context_to_fit(weights: u64, kv_per_token: Option<u64>, trained: Option<u64>, budget: &Budget) -> u32 {
    let trained = trained.and_then(|trained| u32::try_from(trained).ok()).unwrap_or(u32::MAX);
    let most = MOST_CONTEXT.min(trained).max(LEAST_CONTEXT.min(trained));
    let Some(per_token) = kv_per_token else {
        return UNSIZED_CONTEXT.min(most);
    };
    let mut context = most.next_power_of_two();
    while context > most {
        context /= 2;
    }
    while context > LEAST_CONTEXT && fit(need(weights, Some(per_token), context), budget) != Fit::Fits {
        context /= 2;
    }
    context.min(most)
}

pub fn fit(need: u64, budget: &Budget) -> Fit {
    let room = budget.room();
    if need <= room / 10 * 9 {
        Fit::Fits
    } else if need <= room {
        Fit::Tight
    } else {
        Fit::TooBig
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTED: &str = "\
ggml_cuda_init: found 1 CUDA devices:
  Device 0: NVIDIA GeForce RTX 5090 Laptop GPU, compute capability 12.0, VMM: yes
Available devices:
  CUDA0: NVIDIA GeForce RTX 5090 Laptop GPU (24463 MiB, 23860 MiB free)
  Vulkan0: AMD Radeon 8060S Graphics (RADV GFX1151) (98304 MiB, 97000 MiB free)
";

    #[test]
    fn devices_are_read_from_their_lines_only() {
        let devices = parse_devices(LISTED);
        assert_eq!(devices.len(), 2, "{devices:?}");
        assert_eq!(devices[0].name, "CUDA0");
        assert_eq!(devices[0].description, "NVIDIA GeForce RTX 5090 Laptop GPU");
        assert_eq!(devices[0].total, 24463 * MIB);
        assert_eq!(devices[0].free, 23860 * MIB);
        assert_eq!(devices[1].description, "AMD Radeon 8060S Graphics (RADV GFX1151)");
        assert_eq!(parse_devices("Available devices:\n"), []);
    }

    #[test]
    fn the_context_is_the_most_that_fits() {
        let gpu = Budget { devices: parse_devices(LISTED)[..1].to_vec(), memory: 64 << 30 };
        let kv = 48 * 4 * 192 * 2;
        assert_eq!(
            context_to_fit(18_556_689_568, Some(kv), Some(262_144), &gpu),
            32_768,
            "on 24 GB an 18.6 GB model has room for 32k"
        );
        assert_eq!(context_to_fit(17_000_000_000, Some(kv), Some(262_144), &gpu), 65_536, "a 17 GB one for 64k");
        assert_eq!(context_to_fit(1 << 30, Some(kv), Some(262_144), &gpu), 131_072, "never past the most Silt starts with");
        assert_eq!(context_to_fit(1 << 30, Some(kv), Some(4096), &gpu), 4096, "never past what it was trained for");
        assert_eq!(context_to_fit(30 << 30, Some(kv), Some(262_144), &gpu), 8192, "the least, when nothing fits");
        assert_eq!(context_to_fit(1 << 30, None, Some(262_144), &gpu), 32_768, "unsized until its header is read");
        assert_eq!(
            context_to_fit(1 << 30, Some(kv), Some(40_000), &gpu),
            32_768,
            "a power of two within what it was trained for"
        );
    }

    #[test]
    fn a_model_fits_tight_or_not() {
        let gpu = Budget { devices: parse_devices(LISTED)[..1].to_vec(), memory: 64 << 30 };
        let qwen = need(18_556_689_568, Some(48 * 4 * 192 * 2), 32_768);
        assert_eq!(fit(qwen, &gpu), Fit::Fits);
        assert_eq!(fit(need(18_556_689_568, Some(48 * 4 * 192 * 2), 131_072), &gpu), Fit::TooBig);
        assert_eq!(fit(gpu.room() - 1, &gpu), Fit::Tight);
        let cpu = Budget { devices: Vec::new(), memory: 32 << 30 };
        assert_eq!(cpu.room(), 32 << 30);
    }
}
