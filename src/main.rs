use cpal::traits::{DeviceTrait, HostTrait};
use klopfer::Klopfer;
use nih_plug::prelude::*;

fn main() {
    let mut args: Vec<String> = std::env::args().collect();
    if let Some(rate) = device_sample_rate(&args) {
        eprintln!("Using the output device's sample rate of {rate} Hz");
        args.extend(["--sample-rate".to_owned(), rate.to_string()]);
    }
    nih_export_standalone_with_args::<Klopfer, _>(args);
}

/// nih-plug's CoreAudio/WASAPI backend aborts when the device runs at a different rate than the
/// requested one (48 kHz by default), so match the device unless a rate was given explicitly.
fn device_sample_rate(args: &[String]) -> Option<u32> {
    if arg_value(args, "-r", "--sample-rate").is_some() {
        return None;
    }
    // These backends ignore the sample rate option
    if matches!(
        arg_value(args, "-b", "--backend").as_deref(),
        Some("jack" | "dummy")
    ) {
        return None;
    }

    let host = cpal::default_host();
    let device = match arg_value(args, "", "--output-device") {
        Some(name) => host
            .output_devices()
            .ok()?
            .find(|d| d.name().is_ok_and(|n| n == name))?,
        None => host.default_output_device()?,
    };
    Some(device.default_output_config().ok()?.sample_rate().0)
}

/// Finds an option's value in the forms `-x value`, `-xvalue`, `--long value` and `--long=value`.
fn arg_value(args: &[String], short: &str, long: &str) -> Option<String> {
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        if arg == long || (!short.is_empty() && arg == short) {
            return iter.next().cloned();
        }
        if let Some(v) = arg.strip_prefix(long).and_then(|v| v.strip_prefix('=')) {
            return Some(v.to_owned());
        }
        if let Some(v) = arg
            .strip_prefix(short)
            .filter(|v| !short.is_empty() && !v.is_empty())
        {
            if !arg.starts_with("--") {
                return Some(v.to_owned());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::arg_value;

    fn args(s: &str) -> Vec<String> {
        std::iter::once("bin")
            .chain(s.split_whitespace())
            .map(String::from)
            .collect()
    }

    #[test]
    fn parses_option_forms() {
        assert_eq!(
            arg_value(&args("-r 44100"), "-r", "--sample-rate").as_deref(),
            Some("44100")
        );
        assert_eq!(
            arg_value(&args("-r44100"), "-r", "--sample-rate").as_deref(),
            Some("44100")
        );
        assert_eq!(
            arg_value(&args("--sample-rate=96000"), "-r", "--sample-rate").as_deref(),
            Some("96000")
        );
        assert_eq!(
            arg_value(&args("-b dummy"), "-b", "--backend").as_deref(),
            Some("dummy")
        );
        assert_eq!(
            arg_value(&args("-b core-audio -l 2"), "-r", "--sample-rate"),
            None
        );
    }
}
