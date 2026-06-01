# Golden vectors

This directory holds deterministic `(input, expected)` reference pairs used
as cross-backend oracles. Each file follows the schema:

```json
{
  "op": "<operation_name>",
  "version": "1.0",
  "note": "How the reference was produced",
  "inputs": { /* named inputs with shape + data */ },
  "expected": { "shape": [...], "data": [...] },
  "tolerance": { "rtol": 1e-5, "atol": 1e-7 }
}
```

## Generation

References are produced by a Python script (`scripts/gen_golden.py`,
Sprint 2 deliverable) that uses PyTorch as the neutral oracle:

```bash
python scripts/gen_golden.py --op l2_normalize --out tests/golden/l2_normalize/basic_1d.json
```

## Tolerance policy

| Operation family | rtol | atol | Notes |
|------------------|------|------|-------|
| Exact integer (argmax) | 0 | 0 | Index equality is strict. |
| Pure sums / means | 1e-6 | 1e-7 | Simple accumulation. |
| L2 ops, dot | 1e-5 | 1e-7 | Square + sqrt. |
| Transcendental (softmax, gelu) | 1e-5 | 1e-6 | `exp`, `erf`. |
| matmul, attention | 1e-4 | 1e-5 | BLAS reduction order varies. |

## Files added in Sprint 1

- `l2_normalize/basic_1d.json`
- `mean/basic_2d.json`
- `argmax/basic_2d.json`

Sprints 2 and 3 will add 20+ additional golden vectors covering all 18 ops.
