# R124 ordinary-V8 model gate freeze

Date: 2026-08-10  
Status: frozen before any paired model timing

This file authenticates the sole Gate-B model run allowed by the R124
protocol. No eager/hybrid worker timing has been collected.

## Immutable identities

| item | SHA-256 |
|---|---|
| Gate-B runner | `666a6cd668642ec423c6867ff1a51604846087cb429b31e8dbfc0608eda3714a` |
| model generator | `f15e7dffd9cb44f66e2e101def93eab6a830f90513209f2bef66ab72af23f168` |
| R124 protocol | `ddb2a066b8d494c21db202fb35245b6f1988836d98fa1841bd7781cbe5dd735a` |
| A2 dynamic report | `0f424205c3fb9c0d149d151280040738749062a80a81759cad60f741a0e27780` |
| production release Wasm | `d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d` |
| production DBT layout source | `ba4972333a293e37d03b66f0932bc31a3453360077fcb9e5e2ba1766a4811360` |
| production DBT emitter source | `b5e9c11ec1bfa1e92245e6bac003c4af0c6bac4b813d58344d8276940d6a1e99` |
| production Wasm runtime source | `1da35e70bc9c957fd184f1cd9e6772d6d0a7380398aa7fc17369581b53036339` |

The deterministic model outputs are eager `8d57dc9b4920...`, hybrid
`04934d012990...`, shape `29e256bd2e09...`, and schedule
`f3fc0218918d...`, as frozen in the protocol and asserted by the runner.

## Immutable decision

Run exactly 15 alternating fresh-process pairs on CPUs 8--15 with the work,
spread, artifact, affinity, and output rules in the protocol. Report the old
1.05x/1.02x/0.99x proxy thresholds without using them as a large-gain veto.
Proceed to natural optimized-native shape unless the 95% upper bound is below
parity or an integrity/shape check fails. Do not add pairs, change iteration
counts, alter the bank or schedule, or rerun based on the result.
