# CONTINUATION-369 — contrat des milestones de pull request

Date : 28 octobre 2026. Travail local uniquement sur le checkout courant; aucune lecture de courriel, connexion GitLab, opération distante, dépense de modèle ni modification de production.

## Question vérifiée

Forgejo v17.0.0 émet les changements de milestone des pull requests sous l’en-tête wire `pull_request`, avec les actions `milestoned`/`demilestoned` et un objet PR contenant `milestone`. Le runner Python ne validait ni ne retournait ce champ. Le reconciler Rust projetait déjà le milestone du GET courant pour les deux types de ressources, mais le contrat de cette tranche n’avait pas de cas PR.

Source officielle vérifiée dans le clone scratch au commit `0d791421f85ca13a63b665053d81ea765fbbc571` : `services/webhook/notifier.go::IssueChangeMilestone` lignes 561–588 branche sur `issue.IsPull`, choisit les deux actions et crée un `PullRequestPayload` avec `convert.ToAPIPullRequest`; `modules/webhook/type.go::HookEventType.Event` lignes 68–70 mappe le type à `pull_request`; `modules/structs/pull.go::PullRequest.Milestone` ligne 19 porte `json:"milestone"`. Les blobs source propres à ce commit sont hachés dans `source-sha256.json`; `git diff --quiet HEAD` a confirmé que ces trois fichiers ciblés du scratch ne sont pas modifiés. Le scratch global reste sale (64 entrées de statut) et cet examen est limité à ces symboles/fichiers; aucun code Forgejo n’est copié.

## Test-first et changement local

Commande rouge :

```text
python3 -m unittest discover -s tests -p 'test_forgejo_emitted_webhook_e2e.py' -k ParseLivePullRequestDeliveryRecordTests -v
```

Résultat initial : 23 tests, 4 refus attendus non levés et 2 erreurs `KeyError` car `payload_milestone` n’existait pas. Log : `python-parser-red.log`.

Le parseur exige maintenant le champ `pull_request.milestone` pour ces deux actions, valide l’identité bornée `{id,title,state}` avec le normaliseur existant, exige un objet pour `milestoned` et `null` pour `demilestoned`, et retourne `payload_milestone` comme observation du payload. Cette donnée n’est jamais présentée comme l’état canonique.

Le contrat Rust ajoute deux cas fixture : le GET courant d’une PR écrase un milestone webhook volontairement périmé et distingue objet courant, null explicite et champ absent (`unknown`); un milestone API mal formé refuse le snapshot canonique.

## Résultats

- Classe Python ciblée après correction : 23/23 (`python-parser-green.log`).
- Module Python complet : 163/163 (`python-module-final.log`).
- `webhook_reconcile_contract` : 98/98 (`rust-contract-final.log`), dont les deux nouveaux cas PR.
- `cargo fmt --all -- --check`, `py_compile` des deux fichiers Python et `git diff --check` : succès, commandés dans cette reprise.
- `python3 scripts/verify_cargo_license_state.py` : `metadata_policy_verified`; licence et fichier de licence restent nuls, publication désactivée, droits de redistribution non évalués (`cargo-license-state.json`).

Hashes des sources produit et blobs upstream : `source-sha256.json`. Environnement, version du checkout et disponibilité des outils : `environment.txt`.

## Limites

Tests unitaires/contrats avec fixtures, pas une émission native PR milestone, un receiver→journal→CLI composé, ni une réconciliation contre serveur Forgejo vivant. À cette reprise, `forgejo`, `go` et `git-lfs` ne sont pas dans le `PATH`; le port local Forgejo 3130 n’a pas de listener observé. Le prochain lot devrait composer un événement synthétique via le receiver/CLI; il ne pourra pas être décrit comme un événement Forgejo live. Aucun AgentOS/LLM ni gain comparatif n’est testé. Le projet reste incomplet.
