# CONTINUATION-456 — parent frais invalide après 404 de commentaire PR supprimé

Date : 29 octobre 2026. Worktree local : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`. Le worktree était déjà fortement modifié. Aucun changement préexistant n’a été annulé, écrasé, commité ou publié.

## Comportement vérifié

Un webhook signé de suppression de commentaire PR traverse le vrai receiver et l’inbox durable. La première réponse PR est valide; le GET du commentaire retourne un 404; le GET frais de la PR retourne un objet fermé/fusionné, mais omet `draft`. Le test contrat refuse `InvalidCurrentStateField("draft")` après les lectures PR → commentaire → PR, au lieu de transformer le 404 enfant en `comment.absent`.

Le test du vrai `forgejo-reconcile-local` sous Bubblewrap réseau/PID privés confirme exactement trois GET : `GET /pulls/24`, `GET /issues/comments/916`, puis `GET /pulls/24`. Il n’y a aucun en-tête Authorization. Le résultat est `inconclusive/invalid_current_state` avec `field=draft`, `upstream_read_attempts=3`, sans état canonique. Ni le corps du 404 ni le texte du webhook ne sont renvoyés; les octets de l’inbox restent identiques.

- Test de contrat ciblé : **1/1**.
- `webhook_reconcile_contract` : **113/113**.
- Processus isolé : **1/1**.
- `cargo test --locked --offline --all-targets -- --test-threads=1` : **39 cibles, 793 réussis, 0 échec, 216 ignorés**. Les tests processus marqués `ignore` sont exécutés séparément via le runner Bubblewrap.

Les sorties sont `contract-focused.log`, `reconcile-contract.log`, `process-e2e.log` et `all-targets.log`; les compteurs agrégés sont dans `test-summary.json`.

## Portée source, versions et intégrations

Le client lit l’API `GET /repos/{owner}/{repo}/pulls/{index}`. L’inventaire officiel existant fixe Forgejo v17.0.0 au commit `0d791421f85ca13a63b665053d81ea765fbbc571` et cite la route et le DTO PR dans `docs/source-inventory.md:286`. Ce lot n’a pas modifié le code runtime ni relu/copier du code Forgejo.

Outils : Python 3.12.3, Cargo 1.95.0, rustc 1.95.0, Git 2.43.0, Bubblewrap 0.9.0. La vérification Cargo maintient `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, redistribution `not_assessed`, SPDX `NOASSERTION`. Aucune licence produit n’est choisie. Rustfmt, `git diff --check`, compilation syntaxique du runner Python et l’audit des liens des documents canoniques passent; le rapport conserve leurs sorties.

Aucun AgentOS ou modèle n’a été intégré ni appelé. La réévaluation locale OPSYS existante conclut qu’aucun runtime autorisé compatible avec `AgentRuntimeV1` n’est démontré; `FixtureRuntime` demeure le substitut de test clairement identifié ([réévaluation](../opsys-runtime-interface-reassessment-001/local-run.md)). Aucun coût, courriel, GitLab, service externe, secret, publication ou changement de production n’a été impliqué.

## Reproductibilité et limites

Les commandes et les hashes des six fichiers touchés sont dans `commands.txt` et `source-sha256.txt`. Le test utilise un unique champ absent sur une fixture HTTP loopback (`n=1`); il n’établit pas le comportement d’une instance Forgejo live, une panne transitoire lors du parent frais, une garantie atomique des lectures, une intégration LLM/AgentOS ou un gain comparatif.

Les manifestes SHA append-only worktree, projet et paquet sont actualisés par `append_manifests.py`; les préfixes antérieurs sont dans `manifest-prefix-before.json`. Prochaine tranche locale : répondre `503 Retry-After: 0` au GET frais du parent après le 404 enfant, puis vérifier que `absent` n’est produit qu’après un readback parent réussi.
