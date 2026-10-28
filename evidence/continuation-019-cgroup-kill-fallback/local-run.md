# CONTINUATION-019-CGROUP-KILL-FALLBACK-001

## Résultat

Le nettoyage forcé du superviseur MCP avait un chemin incomplet : si l'ouverture ou l'écriture de `cgroup.kill` échouait, il émettait un avertissement puis tuait le groupe hôte. Il existe maintenant un repli borné par le même gestionnaire systemd utilisateur. `verifier_scope_unit` n'accepte qu'une unité ASCII `run-*.scope` sous `user.slice/user-<uid>.slice/user@<uid>.service/app.slice`. Le repli vérifie le propriétaire/mode du runtime dir et le propriétaire/type du socket D-Bus, construit un environnement réduit et appelle seulement `systemctl --user kill --kill-whom=all --signal=SIGKILL <unité>` avec un délai de deux secondes.

Si l'accès cgroup et le bus utilisateur validé sont tous deux indisponibles, la fonction renvoie `False` et écrit un diagnostic explicite; le superviseur ne prétend pas que le scope a été nettoyé. Le processus reste borné par le chemin d'arrêt préexistant. Le statut d'arrêt reste non nul.

## Preuves

- Deux tests Python couvrent le repli vers une unité exacte et le refus diagnostiqué quand le bus validé manque : [journal](fallback-unit-tests.log).
- Un troisième test refuse un scope étranger avant tout accès cgroup ou appel systemd.
- Smoke local avec `systemd-run --user --scope` et un `sleep` jetable : le test injecte `PermissionError` uniquement pour `cgroup.kill`, le repli systemd tue le scope généré pour ce test, et le launcher finit par `-9` : [journal](fallback-systemd-smoke.log), [script](fallback-systemd-smoke.py). Aucun autre scope n'est ciblé.
- Suite Python du superviseur : 13/13 réussis, [journal](python-supervisor-suite.log).
- Cancellation du superviseur avec le vrai host Rust : 1/1, cgroup vide sans avertissement : [journal](cancellation-focused.log).
- `cargo test --locked --offline --all-targets -- --test-threads=1` : 550 réussis, 0 échec, 76 ignorés, 34 groupes : [journal](rust-all-targets.log). Les tests ignorés ne sont pas exécutés.
- `cargo fmt --all -- --check`, `git diff --check` et la garde de métadonnées Cargo passent : [rustfmt](rustfmt.log), [diff](diff-check.log), [cargo](cargo-license-state.log).

## Environnement et limites

Le test système s'exécute localement sur Linux aarch64, Python 3.12.3 et systemd 255 avec hiérarchie unifiée cgroup v2; aucune élévation de privilèges n'est ajoutée. Il prouve le chemin de repli sur ce bus utilisateur, pas sur d'autres configurations systemd/D-Bus. Si le repli échoue, le diagnostic est stderr et l'arrêt demeure en erreur non nulle; le cgroup peut persister si le noyau et le gestionnaire refusent tous deux l'arrêt. Aucun Forgejo live, LLM, AgentOS, coût, service externe, production ou publication n'est impliqué.

Cargo reste sans licence choisie (`license=null`, `license_file=null`, `publish=[]`, droits `not_assessed`). L'arbre existant reste sale; aucun commit n'est créé et aucun fichier préexistant n'est nettoyé. La mission globale est incomplète : le parcours LLM et la comparaison appariée restent non démontrés. Suite : définir si un statut de sortie distinct est nécessaire pour différencier « cancellation terminée » d'« arrêt de scope impossible », puis poursuivre l'audit ciblé des writers/merge sans répéter les cas archivés.
