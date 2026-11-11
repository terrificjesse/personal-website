"""Merge `load.py --latencies-out` files from parallel processes into one summary.

One Python process tops out near 155 requests/second (see docs/PUBLIC_API_STRESS_TEST.md), so
load above that has to come from several, and their percentiles have to be computed from the
pooled samples rather than averaged.
"""

import json
import sys
from collections import Counter

from load import summarize

latencies, statuses, elapsed = [], Counter(), 0.0
for path in sys.argv[1:]:
    with open(path) as f:
        run = json.load(f)
    latencies += run["latencies"]
    statuses.update(run["statuses"])
    elapsed = max(elapsed, run["elapsed"])
print(json.dumps(summarize(f"merged x{len(sys.argv) - 1}", latencies, statuses, elapsed)))
