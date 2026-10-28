# CONTINUATION-284 — contrôler les actions du webhook `issues`

Date : 27 octobre 2026. Worktree de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le checkout était déjà fortement modifié avec des fichiers non suivis. Ce lot n’a ni nettoyé, ni réinitialisé, ni commité ou publié le dépôt.

## Source upstream et lacune constatée

Forgejo officiel v17.0.0 a été audité dans le scratch local `/tmp/forgejo-v17-base-cas-poc` au commit `0d791421f85ca13a63b665053d81ea765fbbc571`. `modules/structs/hook.go` définit l’enum partagé `HookIssueAction`; `services/webhook/notifier.go` émet l’en-tête `issues` pour `opened` (`NewIssue`, autour de la ligne 283), `closed`/`reopened` (changement d’état d’une issue, autour des lignes 261–265) et `edited` (titre/corps, autour des lignes 211–216 et 346–350). Les valeurs plus larges de l’enum sont partagées par d’autres en-têtes et ne sont pas autorisées pour `issues`. Les deux fichiers upstream examinés sont propres dans le scratch; leurs empreintes et leurs lignes concernées sont conservées dans `upstream-source-sha256.txt`. Aucun code Forgejo n’a été copié.

Dans le code local, `validate_event_payload_kind` ne contrôlait pas l’action de `issues`, et son contrat ne la déclarait pas requise. Comme `event_resource_ref` est appelé pendant l’ingestion puis par le readback, une livraison signée sans `action`, de mauvais type ou portant une action d’un autre en-tête pouvait être journalisée et présentée à la sélection de mission.

## Changement

`SUPPORTED_EVENT_ACTION_CONTRACTS` porte maintenant les listes autorisées pour `issues` (`opened`, `closed`, `reopened`, `edited`) et `issue_comment` (`created`, `edited`, `deleted`). Le validateur et l’API de capacités consultent la même donnée; absence, type incorrect ou valeur hors liste retourne `InvalidEventPayloadField("action")`. Le handler HTTP renvoie `400 event_payload_invalid` avec `details.field_path = payload.action` et la livraison n’est pas ajoutée au journal.

Le contrat événementiel v5 expose ces listes. Le receiver publie les enveloppes de capacités v1–v5 et passe au schéma externe 6; le CLI reconciler publie v1–v5 sous `forgejo-reconcile-capabilities-v16`. Les anciennes enveloppes v1–v4 restent présentes. L’admission continue d’effectuer le readback courant de l’issue; une action admise n’est ni une preuve d’état canonique ni une autorisation de fusion.

## Reproduction et vérification

Avant le changement, le test HTTP avec HMAC valide obtenait `202` et journalisait une issue sans action, au lieu du refus attendu `400`. Le test de readback retournait aussi `Ok(ResourceRef { kind: Issue, ... })` pour cette livraison. Ces deux rouges sont décrits dans `red-diagnostic.txt`.

Après correction :

- le test de handler accepte les quatre actions de Forgejo v17 et refuse l’action absente, numérique ou `assigned`; il vérifie le chemin de champ et l’absence de lignes journalisées pour chaque refus;
- le test de readback applique la même liste; le test `issue_comment` conserve aussi un refus actionnable pour une action inconnue;
- le test du vrai CLI et le test du processus receiver vérifient la publication de `event_contracts_v5`;
- `cargo test --locked --offline --all-targets` passe **692 tests, 0 échec, 151 ignorés**, en 37 suites. Les tests ignorés ne sont pas exécutés par `--all-targets`;
- `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` passent. Le readback Cargo reste `license=null`, `license_file=null`, `publish=[]`, droits de redistribution `not_assessed`, `NOASSERTION`.

La première passe globale a relevé trois fixtures locales qui représentaient une livraison `issues` sans le champ désormais requis. Elles ont été alignées (`action=opened`) sans modifier le comportement runtime; les sorties de ces échecs initiaux sont archivées séparément. Le premier ajout au grand `json!` de capacités avait également dépassé la limite de récursion de macro; le champ v5 est désormais sérialisé puis inséré dans l’objet après sa construction, sans relèvement global de limite.

## Portée et limites

La réception HMAC est vérifiée via le vrai handler Rust et un journal local, mais ce nouveau refus n’a pas été exercé dans un nouveau test réseau de processus sandboxé, ni contre Forgejo live. L’examen upstream porte seulement sur les symboles cités dans le dépôt v17.0.0 exact; il ne certifie pas les versions ultérieures. Toutes les quatre actions restent admises syntaxiquement; ce lot ne spécifie pas lesquelles déclenchent une mission. Il ne démontre ni une exécution LLM/AgentOS ni un gain comparatif. La licence du produit n’est pas sélectionnée et les droits de redistribution restent non évalués. La mission globale est incomplète.

Prochaine action locale : poursuivre la preuve de la frontière de processus/réseau entre l’agent, les routes API/Web/`receive-pack` et le credential de fusion, puis traiter les critères de bout en bout et de comparaison encore ouverts. Sorties, versions et empreintes : `commands.txt`, `toolchain.txt`, `all-targets.log`, `red-diagnostic.txt`, `source-sha256.txt`, `upstream-source-sha256.txt` et `SHA256SUMS`.
