"""Run the real provider sysinfo initializer and its tests without a shell/UI.

Usage: python tools/dev/provider-system-tests.py [--benchmark]
Requires cached sysinfo 0.30.13 and its dependencies; Cargo stays offline,
in the debug profile, with one build job and one test thread.
"""

import argparse
from pathlib import Path
import re
import subprocess
import tempfile


BENCHMARK = r"""
#[test]
fn initialization_comparison() {
  use std::time::Instant;

  fn measure(create: fn() -> sysinfo::System) -> f64 {
    let start = Instant::now();
    let system = create();
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    std::hint::black_box(&system);
    elapsed // System is dropped after the elapsed time was captured.
  }

  let old = sysinfo::System::new_all();
  let new = create_provider_system();
  println!("processes: old={}, new={}; CPUs={}, RAM={} bytes, swap={} bytes",
    old.processes().len(), new.processes().len(), new.cpus().len(),
    new.total_memory(), new.total_swap());
  drop((old, new)); // Warm both paths before collecting samples.

  let mut old_times = Vec::new();
  let mut new_times = Vec::new();
  for sample in 0..11 {
    // Alternate order to reduce order and cache bias.
    if sample % 2 == 0 {
      old_times.push(measure(sysinfo::System::new_all));
      new_times.push(measure(create_provider_system));
    } else {
      new_times.push(measure(create_provider_system));
      old_times.push(measure(sysinfo::System::new_all));
    }
  }
  old_times.sort_by(f64::total_cmp);
  new_times.sort_by(f64::total_cmp);
  println!("debug initialization, 11 samples each, milliseconds: old median={:.3} range={:.3}..{:.3}; new median={:.3} range={:.3}..{:.3}",
    old_times[5], old_times[0], old_times[10], new_times[5], new_times[0], new_times[10]);
}
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--benchmark", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    source = (root / "shell/packages/desktop/src/providers/provider_manager.rs").read_text(
        encoding="utf-8"
    )
    # Compile the initializer and inline tests verbatim, using the real sysinfo
    # dependency. This avoids Tauri/Win32 app setup and unrelated shell tests.
    initializer = re.search(
        r"^fn create_provider_system\(\) -> sysinfo::System \{.*?^\}",
        source, re.MULTILINE | re.DOTALL,
    )
    tests = re.search(
        r"^#\[cfg\(test\)\]\s*\nmod tests \{.*\}\s*\Z",
        source, re.MULTILINE | re.DOTALL,
    )
    if initializer is None or tests is None:
        raise RuntimeError("provider initializer or inline regression tests not found")

    with tempfile.TemporaryDirectory(prefix="ll-provider-system-") as temp:
        crate = Path(temp)
        (crate / "src").mkdir()
        (crate / "Cargo.toml").write_text(
            '[package]\nname = "ll-provider-system-tests"\nversion = "0.0.0"\n'
            'edition = "2021"\n[lib]\ndoctest = false\n'
            '[dependencies]\nsysinfo = "=0.30.13"\n',
            encoding="utf-8",
        )
        (crate / "src/lib.rs").write_text(
            initializer.group() + "\n" + tests.group() + (BENCHMARK if args.benchmark else ""),
            encoding="utf-8",
        )
        result = subprocess.run(
            ["cargo", "test", "--offline", "--jobs", "1", "--manifest-path",
             str(crate / "Cargo.toml"), "--target-dir",
             str(Path(tempfile.gettempdir()) / "ll-provider-system-tests-target"),
             "--", "--test-threads=1", "--nocapture"],
            cwd=crate,
        )
        return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
