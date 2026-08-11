#!/usr/bin/env python3
"""Summarize UHD RF surveys and reject offset-dependent receiver images."""

import argparse
import math
import sys
from contextlib import ExitStack
from dataclasses import dataclass, field
from pathlib import Path
from statistics import median
from typing import Iterable, TextIO


@dataclass
class LinearAccumulator:
    """Keep a compensated sum and maximum for one LO path."""

    count: int = 0
    total: float = 0.0
    correction: float = 0.0
    maximum: float = 0.0

    def add(self, value: float) -> None:
        """Add one linear-power measurement using Kahan summation."""
        adjusted = value - self.correction
        updated = self.total + adjusted
        self.correction = (updated - self.total) - adjusted
        self.total = updated
        self.maximum = max(self.maximum, value)
        self.count += 1

    def mean(self) -> float:
        """Return the arithmetic mean in linear power units."""
        return self.total / self.count


@dataclass
class PowerAccumulator:
    """Keep separate measurements for positive and negative LO offsets."""

    paths: dict[int, LinearAccumulator] = field(default_factory=dict)

    def add(self, value: float, lo_offset: float) -> None:
        """Add one measurement to its zero, positive, or negative LO path."""
        sign = 0 if lo_offset == 0.0 else (1 if lo_offset > 0.0 else -1)
        self.paths.setdefault(sign, LinearAccumulator()).add(value)

    @property
    def count(self) -> int:
        """Return the total observation count across all LO paths."""
        return sum(path.count for path in self.paths.values())

    def uses_image_rejection(self) -> bool:
        """Return whether both nonzero LO-offset paths are present."""
        return -1 in self.paths and 1 in self.paths

    def mean(self, reject_images: bool) -> float:
        """Return the pooled or two-path image-rejected linear mean."""
        if reject_images and self.uses_image_rejection():
            return min(self.paths[-1].mean(), self.paths[1].mean())
        return math.fsum(path.total for path in self.paths.values()) / self.count

    def maximum(self, reject_images: bool) -> float:
        """Return the pooled or two-path image-rejected linear maximum."""
        if reject_images and self.uses_image_rejection():
            return min(self.paths[-1].maximum, self.paths[1].maximum)
        return max(path.maximum for path in self.paths.values())


def parse_args() -> argparse.Namespace:
    """Parse command-line options."""
    parser = argparse.ArgumentParser(
        description=(
            "Summarize uhd_rf_survey output by frequency. Powers are converted "
            "from dBFS/Hz to linear units before averaging; the maximum is "
            "also retained."
        )
    )
    parser.add_argument(
        "input",
        type=Path,
        help="survey file, or - to read from standard input",
    )
    parser.add_argument(
        "-o",
        "--output",
        type=Path,
        help="averaged spectrum file (default: standard output)",
    )
    parser.add_argument(
        "--summary-only",
        action="store_true",
        help="print the band summary without writing an averaged spectrum",
    )
    parser.add_argument(
        "--bin-width",
        type=float,
        help="frequency-bin width in Hz (normally inferred from the input)",
    )
    parser.add_argument(
        "--no-image-rejection",
        action="store_true",
        help=(
            "pool positive and negative LO-offset observations instead of "
            "rejecting one-sided receiver images"
        ),
    )
    args = parser.parse_args()
    if args.bin_width is not None and args.bin_width <= 0.0:
        parser.error("--bin-width must be greater than zero")
    return args


def read_survey(lines: Iterable[str]) -> tuple[dict[float, PowerAccumulator], int]:
    """Read survey rows, retaining only one accumulator per frequency."""
    bins: dict[float, PowerAccumulator] = {}
    rows = 0
    for line_number, line in enumerate(lines, 1):
        fields = line.partition("#")[0].split()
        if not fields:
            continue
        if len(fields) not in (3, 4):
            # this is probably the last line in a file still being written, so break here.
            break
            #raise ValueError(f"line {line_number}: expected three or four columns")
        try:
            frequency = float(fields[1])
            power_dbfs_per_hz = float(fields[2])
            lo_offset = float(fields[3]) if len(fields) == 4 else 0.0
        except ValueError as error:
            raise ValueError(f"line {line_number}: invalid numeric value") from error
        if (
            not math.isfinite(frequency)
            or not math.isfinite(power_dbfs_per_hz)
            or not math.isfinite(lo_offset)
        ):
            continue
            #raise ValueError(f"line {line_number}: values must be finite")
        try:
            linear_power = 10.0 ** (power_dbfs_per_hz / 10.0)
        except OverflowError as error:
            raise ValueError(f"line {line_number}: power is out of range") from error
        bins.setdefault(frequency, PowerAccumulator()).add(linear_power, lo_offset)
        rows += 1
    if not bins:
        raise ValueError("input contains no survey rows")
    return bins, rows


def linear_to_db(linear_power: float) -> float:
    """Convert linear power to decibels, preserving an underflowed zero."""
    if linear_power == 0.0:
        return -math.inf
    return 10.0 * math.log10(linear_power)


def infer_bin_width(frequencies: list[float]) -> float | None:
    """Infer the nominal FFT-bin spacing from adjacent frequencies."""
    if len(frequencies) < 2:
        return None
    return median(b - a for a, b in zip(frequencies, frequencies[1:]))


def write_spectrum(
    output: TextIO,
    frequencies: list[float],
    bins: dict[float, PowerAccumulator],
    reject_images: bool,
) -> None:
    """Write the mean, maximum, and count for each frequency."""
    print(
        "# frequency_hz average_power_dbfs_per_hz "
        "maximum_power_dbfs_per_hz observations",
        file=output,
    )
    for frequency in frequencies:
        accumulator = bins[frequency]
        print(
            f"{frequency:.6f} "
            f"{linear_to_db(accumulator.mean(reject_images)):.9f} "
            f"{linear_to_db(accumulator.maximum(reject_images)):.9f} "
            f"{accumulator.count}",
            file=output,
        )


def print_summary(
    frequencies: list[float],
    bins: dict[float, PowerAccumulator],
    rows: int,
    bin_width: float | None,
    reject_images: bool,
    output: TextIO,
) -> None:
    """Print the equal-frequency-weighted mean and integrated band power."""
    means = [bins[frequency].mean(reject_images) for frequency in frequencies]
    mean_density = math.fsum(means) / len(means)
    counts = [bins[frequency].count for frequency in frequencies]

    print(f"Rows read: {rows}", file=output)
    print(f"Frequency bins: {len(frequencies)}", file=output)
    print(f"Observations per bin: {min(counts)} to {max(counts)}", file=output)
    rejected_bins = sum(
        bins[frequency].uses_image_rejection() for frequency in frequencies
    )
    if reject_images and rejected_bins:
        print(f"Two-path image rejection: {rejected_bins} bins", file=output)
    print(
        f"Mean power density: {linear_to_db(mean_density):.6f} dBFS/Hz",
        file=output,
    )

    if bin_width is None:
        print(
            "Integrated band power: unavailable (specify --bin-width)",
            file=output,
        )
        return

    covered_bandwidth = len(frequencies) * bin_width
    lower_edge = frequencies[0] - bin_width / 2.0
    upper_edge = frequencies[-1] + bin_width / 2.0
    integrated_power = math.fsum(means) * bin_width
    print(f"Nominal bin width: {bin_width:.6f} Hz", file=output)
    print(
        f"Frequency extent: {lower_edge:.6f} to {upper_edge:.6f} Hz",
        file=output,
    )
    print(f"Observed bandwidth: {covered_bandwidth:.6f} Hz", file=output)
    print(
        f"Integrated band power: {linear_to_db(integrated_power):.6f} dBFS",
        file=output,
    )

    expected_bins = round((frequencies[-1] - frequencies[0]) / bin_width) + 1
    if expected_bins > len(frequencies):
        print(
            f"Warning: the frequency extent contains approximately "
            f"{expected_bins - len(frequencies)} missing bins",
            file=sys.stderr,
        )


def main() -> int:
    """Average a survey file and emit its spectrum and band summary."""
    args = parse_args()
    try:
        with ExitStack() as stack:
            input_file = (
                sys.stdin
                if str(args.input) == "-"
                else stack.enter_context(args.input.open(encoding="utf-8"))
            )
            bins, rows = read_survey(input_file)

            frequencies = sorted(bins)
            bin_width = args.bin_width or infer_bin_width(frequencies)
            reject_images = not args.no_image_rejection
            if not args.summary_only:
                output_file = (
                    stack.enter_context(args.output.open("x", encoding="utf-8"))
                    if args.output is not None
                    else sys.stdout
                )
                write_spectrum(output_file, frequencies, bins, reject_images)

        summary_output = (
            sys.stderr
            if not args.summary_only and args.output is None
            else sys.stdout
        )
        print_summary(
            frequencies,
            bins,
            rows,
            bin_width,
            reject_images,
            summary_output,
        )
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
