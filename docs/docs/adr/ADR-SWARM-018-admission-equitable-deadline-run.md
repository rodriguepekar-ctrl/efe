# ADR-SWARM-018 : Admission **équitable** (par-tenant) + **deadline run** coopératif — les trois plafonds, un principe

- **Status** : Accepted · **Date** : 2026-06-17
- **Phase** : la gouvernance des runs comme contrat (après le coût et l'intégrité, le **temps**
  et l'**équité**)
- **Related** : architecture `docs/reports/EFE_Archi_Admission_Deadline_2026-06-17.html` ;
  ADR-SWARM-014 (CP-2 : admission process GLOBALE + I-AK4 sonde de budget coopérative + I-CP1
  largeur neutre) ; ADR-SWARM-006 (la provenance par-agent est le moat) ; ADR-SWARM-017
  (re-provable ≠ reproductible : le point de coupe wall-clock) ; `sizing.rs`
  (`run_waves`/`DeadlineProbe`/`StopReason::Deadline`) ; `deadline.rs` (`DeadlineGuard`) ;
  `sdk.rs` (`run_traced` : sonde coopérative + backstop) ; `state.rs` (`admit`/`AdmissionPermit`) ;
  Lot ADMIT-DEADLINE (artefact `scripts/swarm/EFE_SWARM_ADMITDL_2026-06-17.json`)

## Contexte

Un run d'essaim avait jusqu'ici **deux** plafonds : le **coût** (`Budget` + `BudgetGuard`, I13 ;
arrêt coopératif des vagues via `BudgetProbe`, I-AK4) et la **concurrence** (admission process
ADR-014 CP-2 : un sémaphore FIFO global, 429 + `Retry-After`). Deux trous restaient ouverts.

1. **L'admission était GLOBALE.** Un seul tenant bavard pouvait occuper TOUS les permits et
   affamer les autres — le gap « fairness par tenant » explicitement laissé ouvert en ADR-014.

2. **Aucun plafond de TEMPS.** Un run pouvait être borné en euros et en parallélisme, mais pas en
   durée : seuls existaient des timeouts par-batch (T2/T3), jamais un deadline de run. Or borner
   le temps est piégeux ici : `run_traced` **scelle la provenance APRÈS** la topologie. Un
   `tokio::timeout` naïf qui *drop* la topologie en plein vol semblait détruire la preuve.

**Découverte du build (féconde).** Le `ProvenanceRecorder` est **partagé** via le sink
(`ctx.write_sink`) : tout `AgentCallRecord` déjà écrit y **survit** au drop du futur tranché.
Un backstop `tokio::timeout` ne perd donc PAS la preuve — il scelle le partiel présent, à un
point de coupe simplement plus grossier que l'arrêt coopératif. Le deadline cesse d'être un
dilemme « tuer vs prouver » : on peut tuer **et** prouver.

## Décision

1. **Trois plafonds orthogonaux, un principe** — coût (`Budget`), concurrence (admission), temps
   (deadline) se composent sans interférence, sous la même doctrine : *borner visiblement, sceller
   le partiel, dire ce qui reste reproductible*. Aucune des trois ne touche une cellule, un record
   ou la racine ; chacune ne fait que **tronquer ou refuser**, visiblement.

2. **Le deadline est le JUMEAU TEMPOREL du budget** — une `DeadlineProbe`
   (`Arc<dyn Fn() -> bool>`, `sizing.rs`) greffée au **même point** de la boucle de vagues que la
   `BudgetProbe` : une vague est émise SSI `budget(want) ∧ deadline()`. Le `DeadlineGuard`
   (`deadline.rs`, jumeau atomique du `BudgetGuard`) tient l'instant de départ + la fenêtre ; sa
   sonde `check()` arme un drapeau `tripped` au premier refus (c'est ce drapeau, pas l'horloge
   relue, qui qualifie le run). Zéro nouvelle machinerie de flux.

3. **Coopératif, PAS un `tokio::timeout` naïf** — la sonde coopérative arrête le sizer adaptatif
   sur une **frontière de vague nette** (`StopReason::Deadline`) : le partiel est un préfixe
   propre, jamais une demi-vague. C'est l'étage **fin**, qui couvre les topologies à vagues
   (consensus / generate-verify adaptatifs).

4. **Backstop DUR universel, qui scelle quand même** — `run_traced` enveloppe la topologie d'un
   `tokio::time::timeout(deadline + grâce)` (grâce = `deadline/4` ∈ `[250 ms, 5 s]`). Si une
   topologie **non coopérative** (K fixe, T2/T3) ou un *hang* dépasse, le timeout tranche — et
   `run_traced` **scelle le partiel** présent dans le recorder partagé (le moat survit au drop).
   Universel (toutes topologies), point de coupe arbitraire. Au régime nominal adaptatif, le
   coopératif a déjà arrêté bien avant ce backstop.

5. **Re-provable, PAS reproductible (écho ADR-017)** — le point de troncature dépend du
   **wall-clock** (combien de vagues ont tenu). Un run deadliné est donc **re-prouvable** (R0 :
   re-sceller les records stockés → la racine se reproduit) mais **non reproductible** (R1 : une
   ré-exécution plus lente/rapide tronquera ailleurs). Le statut `RunTermination` le signale :
   `DeadlineExceeded` (coopératif) / `DeadlineHardAbort` (backstop) — partiel mais honnête,
   exactement comme `budget_stopped`. `is_clean()` est désormais `termination == Completed`.

6. **Admission ÉQUITABLE par-tenant** — un cap par-tenant (`admission_per_tenant_max`) superposé
   au sémaphore global. `admit(tenant)` teste l'**équité d'abord** (le tenant a-t-il atteint SON
   quota ? sinon `TenantQuota`), la **capacité ensuite** (reste-t-il un permit global ? sinon
   `GatewayFull`) — sinon un tenant bavard épuiserait le global avant le test d'équité. Le permit
   est un RAII composite (`AdmissionPermit`) : son `Drop` libère le global ET décrémente le
   compteur du tenant. Défaut `max(1, global − 1)` : un **cap, pas une réservation** — il garantit
   qu'aucun tenant ne prend le dernier permit, laissant toujours ≥ 1 slot tourner pour autrui.

7. **Plafonds bornés CÔTÉ SERVEUR** — `deadline_ms` est clampé `[50 ms, EFE_GATEWAY_MAX_DEADLINE_MS]`
   (défaut 5 min) ; le cap par-tenant dérive du global (ou `EFE_GATEWAY_MAX_INFLIGHT_PER_TENANT`).
   Un client ne réserve ni le temps ni la concurrence d'autrui (parité avec le plafond `width`).
   Le 429 distingue la cause (`reason: gateway_full` ⇒ réessayez ; `tenant_quota` ⇒ ralentissez
   VOTRE débit) ; `/metrics` expose le cap par-tenant + les tenants actifs.

8. **Chemin historique strictement préservé** — sans deadline (`None`, défaut), `run_traced`
   appelle `topo.run` nu : zéro `tokio::timeout` enveloppé, aucune sonde branchée, statut
   `Completed`, run `clean`. L'admission par-tenant à `per_tenant = global` redevient l'admission
   globale ADR-014. Additif, jamais une régression.

## Invariants introduits (famille I-AD — 8 invariants, gardiens dédiés ; + socle `sizing.rs`/`deadline.rs`)

- **I-AD1** — *Trois plafonds composables ; le PREMIER atteint arrête et est NOMMÉ* : coût,
  temps et concurrence coexistent ; dans la boucle on émet SSI `budget ∧ deadline`, et le premier
  plafond atteint arrête la vague **en nommant sa cause**. **« Premier » = précédence
  DÉTERMINISTE**, pas « premier dans le temps » : les deux sondes sont consultées **au même
  instant**, avant la même vague — l'ordre est fixé par le code (budget testé avant deadline dans
  la vague ; au niveau run, `hard_abort > deadline > budget > completed`). Gardiens :
  `first_ceiling_wins_and_is_named` (sizing — les DEUX directions `budget`/`deadline` + le cas
  double-plafond) · `run_termination_names_the_winning_ceiling` (SDK — `RunTermination` nomme le
  plafond : `BudgetStopped` vs variant deadline, jamais confondus) · témoins harnais §D/§E.
- **I-AD2** — *Deadline coopératif sur frontière de vague* : la sonde refuse une vague ENTIÈRE
  avant de l'émettre (jumeau de I-AK4) — jamais de demi-vague, zéro sentinelle ; à l'extrême,
  tronque sous `k_min` (le temps prime la borne basse). Gardiens :
  `deadline_probe_stops_cleanly_at_a_wave_boundary` · `deadline_on_first_wave_yields_zero_sample_honestly`.
- **I-AD3** — *Partiel scellé & re-prouvable* : un run tronqué (coopératif OU backstop) scelle ce
  qui a été produit, et le partiel se re-prouve HORS LIGNE (R0). Gardiens (bout-en-bout) :
  `deadline_truncates_adaptive_run_and_seals_a_verifiable_partial` ·
  `hard_backstop_aborts_a_noncooperative_run_yet_still_seals`.
- **I-AD4** — *Re-provable, pas reproductible* (écho ADR-017) : le point de coupe wall-clock rend
  le partiel re-prouvable (R0) mais non reproductible (R1) ; `RunTermination` le signale. Démontré
  par le harnais §B (racine du partiel reproduite hors ligne, statut `deadline_exceeded`).
- **I-AD5** — *Backstop universel, coopératif spécifique* : la sonde couvre l'adaptatif (vagues) ;
  le `tokio::timeout(deadline + grâce)` couvre TOUTE topologie (K fixe, T2/T3, hang) et scelle le
  partiel malgré le drop. Gardien : `hard_backstop_aborts_a_noncooperative_run_yet_still_seals`.
- **I-AD6** — *Admission équitable par-tenant* : un tenant capé par SON quota n'affame pas les
  autres (équité testée avant capacité) ; le global borne le total ; libérer un permit ré-ouvre un
  slot (RAII). Gardien : `admit_enforces_per_tenant_fairness_then_global_capacity`.
- **I-AD7** — *Plafonds bornés serveur* : `deadline_ms` clampé `[50 ms, cap]` ; cap par-tenant
  dérivé du global (`max(1, global−1)`) laissant toujours du souffle. Gardiens :
  `deadline_ms_is_optional_and_clamped_server_side` ·
  `per_tenant_cap_defaults_below_global_to_guarantee_headroom`.
- **I-AD8** — *Chemin historique préservé* : sans deadline → `Completed` + `clean`, zéro surcoût ;
  `deadline_ms` accepté par le contrat HTTP et plombé jusqu'au statut de réponse. Gardiens :
  `run_without_deadline_terminates_completed` ·
  `run_accepts_deadline_ms_and_reports_termination_in_response`.
- **Socle** : `deadline.rs` (3 unités — sonde arme `tripped` à l'expiration ; guard frais ouvert ;
  `hard_limit` encadre la deadline d'une grâce bornée) · `sizing.rs`
  (`run_waves`/`DeadlineProbe`/`StopReason::Deadline`, jumeaux exacts du budget).

## Verdicts des gates (exécutés le jour du build — artefact daté)

- **Lot ADMIT-DEADLINE : GO** (`scripts/swarm/EFE_SWARM_ADMITDL_2026-06-17.json`) — mesures
  RÉELLES (crypto BLAKE3+HMAC+HKDF, timings wall-clock ; agents lents synthétiques, **étiquetés**).
  **§A équité** : `alice` capée à 2 par `tenant_quota` alors que le global avait 1 libre ; `bob`
  obtient ce slot (pas de famine) puis heurte `gateway_full` ; ré-admission après libération.
  **§B coopératif** : consensus adaptatif (gén. 100 ms, deadline 40 ms) → `deadline_exceeded`,
  `k_used = 4 < 24`, **104 ms** (sous la limite dure ~290 ms), partiel **re-prouvé hors ligne (R0)
  avec racine reproduite**. **§C backstop** : consensus K fixe (gén. 500 ms, deadline 20 ms) →
  `deadline_hard_abort` en **273 ms** (≈ deadline + grâce, bien avant les 500 ms du générateur),
  provenance scellée, run renvoyé Ok. **§D témoin** : sans deadline → `completed` + `clean` +
  vérifié.

## Conséquences

### Positives

- **Le temps devient un plafond de première classe**, au même rang que le coût et la concurrence —
  et, comme eux, **visible** (statut `RunTermination`) et **sans perte de preuve**.
- **Le deadline ne sacrifie pas le moat** : un run tronqué reste re-prouvable (R0). C'est l'apport
  conceptuel — la découverte du recorder partagé transforme le backstop d'une perte de preuve en
  un simple point de coupe plus grossier.
- **Plus de famine inter-tenant** : un client bavard est capé par son quota, jamais au détriment
  des autres ; le 429 dit *pourquoi* (réessayer vs ralentir).
- **Quasi zéro primitive nouvelle** : la sonde de deadline est le clone de la sonde de budget ; le
  backstop est un `tokio::timeout` ; l'admission équitable est un compteur par-tenant superposé au
  sémaphore existant. Aucune sémantique touchée (I-CP1 préservé).

### Négatives / risques tenus (honnêteté de portée)

- **Le coopératif ne couvre QUE les topologies à vagues** (consensus / generate-verify adaptatifs).
  Le K fixe, T2 (pipeline) et T3 (hiérarchique) reposent sur le backstop dur — dont le point de
  coupe est arbitraire, et dont le partiel **peut être vide** s'il tombe avant la première écriture
  (mesuré en §C : 0 record, car le consensus K fixe n'écrit `/cand/*` qu'après la jointure des
  générateurs). Le partiel **non vide** est la promesse du coopératif, pas du backstop.
- **Re-provable, pas reproductible** — un run deadliné ne se reproduit pas par re-exécution (R1
  peut tronquer ailleurs). Par conception, signalé par le statut (jamais un faux « non
  reproductible »).
- **Le cap par-tenant est un CAP, pas une réservation** : avec N tenants actifs, il borne la part
  de chacun mais ne pré-alloue rien (impossible — N est dynamique). L'équité stricte (quota = part
  garantie) reste un raffinement futur, hors périmètre.
- **La grâce du backstop est heuristique** (`deadline/4` borné) : un run dont une vague unique
  dépasse `deadline + grâce` sera tranché dur même en adaptatif. Acceptable (le coopératif vise
  des vagues bien plus courtes que la fenêtre).
