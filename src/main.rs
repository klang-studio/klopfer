use cpal::traits::{DeviceTrait, HostTrait};
use klopfer::Klopfer;
use nice_plug::prelude::*;

fn main() {
    let mut args: Vec<String> = std::env::args().collect();
    if let Some((rate, source)) = sample_rate(&args) {
        eprintln!("Using the {source}'s sample rate of {rate} Hz");
        args.extend(["--sample-rate".to_owned(), rate.to_string()]);
    }
    nice_export_standalone_with_args::<Klopfer, _>(args);
}

/// The sample rate to initialize the plugin with, unless one was given explicitly.
///
/// nice-plug's CoreAudio/WASAPI backend aborts when the device runs at a different rate than the
/// requested one (48 kHz by default). Its JACK backend runs at the server's rate but still
/// initializes the plugin with the requested one, which detunes everything.
fn sample_rate(args: &[String]) -> Option<(u32, &'static str)> {
    if arg_value(args, "-r", "--sample-rate").is_some() {
        return None;
    }
    let jack = || jack_sample_rate().map(|rate| (rate, "JACK server"));
    let device = || device_sample_rate(args).map(|rate| (rate, "output device"));
    match arg_value(args, "-b", "--backend").as_deref() {
        Some("dummy") => None,
        Some("jack") => jack(),
        // Auto tries JACK first
        None | Some("auto") => jack().or_else(device),
        Some(_) => device(),
    }
}

fn jack_sample_rate() -> Option<u32> {
    let (client, _) =
        jack::Client::new("klopfer-probe", jack::ClientOptions::NO_START_SERVER).ok()?;
    Some(client.sample_rate())
}

fn device_sample_rate(args: &[String]) -> Option<u32> {
    let host = cpal::default_host();
    let device = match arg_value(args, "", "--output-device") {
        Some(name) => host
            .output_devices()
            .ok()?
            .find(|d| d.description().is_ok_and(|d| d.name() == name))?,
        None => host.default_output_device()?,
    };
    Some(device.default_output_config().ok()?.sample_rate())
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
