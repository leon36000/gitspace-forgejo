# CONTINUATION-234 — readback Actions refuse le statut courant « unknown »

Date : 27 octobre 2026. Worktree local isolé, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; l’arbre déjà modifié a été préservé.

## Résultat

Deux tests de processus exercés par `scripts/run_webhook_reconcile_cli_e2e.py` signent un événement de succès valide, l’admettent via le vrai `forgejo-webhook-local`, puis font répondre la fixture HTTP comme le readback courant du CLI avec `status="unknown"` :

- `actual_reconcile_cli_process_refuses_unknown_current_workflow_run_status` passe 1/1 (143 tests filtrés).
- `actual_reconcile_cli_process_refuses_unknown_current_workflow_job_status_before_parent_read` passe 1/1 (143 tests filtrés).

Dans les deux cas, le CLI réel retourne `inconclusive/invalid_current_state`, `details.field=status`, un seul readback et aucun `canonical_state`. L’inbox reste byte-identical et aucun marqueur privé de fixture ne paraît en sortie. Pour `workflow_job`, l’unique GET est `/actions/jobs/9001`; le CLI refuse avant le GET du run parent.

Cette tranche ne modifie pas la logique de production : `current_workflow_run_snapshot` et `reconcile_workflow_job` validaient déjà le champ par `valid_workflow_run_status`. Elle ajoute une preuve de bout en bout au processus receiver→journal→CLI et confirme un refus observable quand l’API renvoie une valeur de statut non projetable.

## Référence amont et licence

Forgejo officiel est épinglé dans le scratch au commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Le fichier `models/actions/status.go` n’y est pas modifié : le type partagé `Status` couvre `ActionRun` et `ActionRunJob`; `StatusUnknown.String()` produit `"unknown"`. Le contrat local des statuts webhook expose seulement les valeurs de statut couvertes par les événements; cette valeur n’est donc pas un état canonique soutenu. L’examen est borné à ce fichier source et au mapping Rust cité, sans test Forgejo server/runner.

Readback Cargo : `agent-forge-core 0.1.0`, `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits `not_assessed`, métadonnée personnalisée `NOASSERTION`. Aucun champ ou choix de licence n’a été ajouté; `NOASSERTION` n’accorde aucun droit.

## Commandes et vérifications

Commandes isolées, versions, sorties, résultat du garde Cargo et SHA-256 sont dans les fichiers voisins. Le premier `cargo fmt --all -- --check` a signalé trois écarts de formatage dans le nouveau test; `cargo fmt --all` les a corrigés, puis le contrôle final a passé. Sont passés : les deux scénarios ci-dessus, le contrôle de format final, `git diff --check`, `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` et `scripts/verify_cargo_license_state.py`. Les appels Cargo étaient verrouillés/hors réseau. Le processus Bubblewrap utilisait `--unshare-net --unshare-pid` et n’autorisait que le loopback de fixture.

## Portée et limites

L’API est une fixture loopback, pas une instance Forgejo live; aucun runner Actions, LLM, AgentOS ou fournisseur n’est invoqué. Le test couvre uniquement le statut exact `unknown` pour le binaire courant et ne démontre ni toutes les valeurs futures, ni toutes les versions Forgejo, ni la validité d’un verdict de CI. Il renforce le refus d’état incomplet sans établir une amélioration de productivité ou un gain général. Aucun coût, publication, changement de production, donnée privée externe ou élargissement d’accès.

Le projet reste incomplet : comparaison contrôlée, runtime AgentOS/LLM autorisé, autorité de merge atomique contre tous writers et clarification des droits de redistribution restent ouverts.
