# API Reference

!!! note "À venir"
    La référence API complète sera disponible à partir de Sprint 6
    (lancement gRPC + SDK Python, release 0.1.0).

## Endpoints prévus

### gRPC `SwarmService`

| Méthode | Rôle |
|---------|------|
| `Execute` | Lance un essaim synchrone, retourne la réponse complète. |
| `ExecuteStream` | Lance un essaim avec streaming des rounds intermédiaires. |
| `GetTelemetry` | Récupère la télémétrie d'un essaim par `swarm_id`. |
| `Health` | Probe de santé du serveur. |

### Client Python

```python
from efe import EfeClient, EfeConfig

client = EfeClient(endpoint="localhost:50051")
resp = await client.solve("What is 17 * 23?")
print(resp.decoded_text)        # "391"
print(resp.confidence)          # 0.94
print(resp.telemetry.rounds)    # 4
```

### CLI

```bash
efe-cli solve "Your question"
efe-cli bench gsm8k --n 500
efe-cli telemetry <swarm_id>
```
