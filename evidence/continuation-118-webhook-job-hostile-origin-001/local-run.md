# CONTINUATION-118 — traiter l’URL de job du webhook comme une entrée non fiable

Date : 26 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; base HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Les fichiers existants étaient déjà fortement modifiés; aucune modification n’a été commitée, stagée ou publiée.

## Gap et comportement vérifié

CONTINUATION-117 établissait que l’index job du `html_url` HMAC doit correspondre à la réponse API, mais aucun scénario ne vérifiait que son origine étrangère ne serait ni suivie pour les GET ni retournée comme lien canonique. Le test contrat `workflow_job_readback_does_not_project_or_follow_event_url_origin` fournit une URL signée `https://attacker.invalid/.../jobs/1/attempt/1`, un readback API cohérent pour le même ID/run/index mais avec une URL configurée loopback et une tentative plus récente. Il confirme que la projection renvoie l’URL API et le statut courant du job.

Le test processus existant `actual_reconcile_cli_process_reads_workflow_job_and_parent_run_without_runner_details` emploie désormais la même origine hostile dans le webhook. Receiver HMAC réel → journal → CLI réel s’exécutent dans le runner Bubblewrap avec namespaces réseau/PID séparés. La fixture enregistre exactement les deux GET configurés (`actions/jobs/9001`, puis `actions/runs/7001`); aucun `attacker.invalid` ne paraît dans le JSON de sortie; le lien projeté vient de la réponse API et le journal reste inchangé. L’exécution du scénario réussit 1/1 avec 110 filtrés.

Le code ajoute un commentaire local à `workflow_job_index_from_event_url` explicitant la règle : l’URL événementielle n’est qu’un indice d’identité et ne devient ni destination de requête ni lien affiché. Aucun autre comportement produit n’a été modifié.

## Métadonnées de licence

Le readback Cargo réel confirme `agent-forge-core 0.1.0`, `license=null`, `license_file=null`, `publish=[]` et le marqueur de décision `no_license_selected`; les droits de redistribution restent `not_assessed` et l’export SPDX reste `NOASSERTION`. `scripts/verify_cargo_license_state.py` retourne `verified`; `scripts.test_cargo_license_state` passe 9/9. Aucune licence produit n’est attribuée.

## Vérifications

- `cargo test --offline --locked --test webhook_reconcile_contract` — 55 réussis, 0 échec.
- Runner processus `actual_reconcile_cli_process_reads_workflow_job_and_parent_run_without_runner_details` — 1/1; reçu loopback et Bubblewrap dans `process-e2e.log`.
- Garde Cargo + suite dédiée — état `verified`, 9/9 tests.
- `cargo fmt --all -- --check` et `git diff --check` finaux passent. La première vérification rustfmt a signalé uniquement la mise en forme d’une ligne ajoutée; rustfmt ciblé l’a corrigée et les contrôles finaux sont verts.

Commandes, versions, empreintes des sources et readback Cargo filtré sont conservés dans ce dossier et liés par son `SHA256SUMS`.

Le premier instantané des préfixes de manifeste a échoué avant toute écriture d’index : le chemin relatif du manifeste projet pointait vers le mauvais parent. Cette erreur de préparation est conservée dans `manifest-prefix-bootstrap-error.log`; le chemin a été corrigé avant l’append. L’append final a ajouté 17 éléments au manifeste local, 21 à celui du worktree et 22 à celui du projet; le vérificateur confirme les trois préfixes préservés, les 17 fichiers locaux et le lien courant du manifeste parent.

## Portée et limites

Le payload est synthétique, bien que signé par la clé du test; il n’a pas été émis par Forgejo live. L’API est une fixture loopback sous `bwrap --unshare-net`, pas un runner réel; les deux GET attendus sont capturés côté fixture mais aucun enregistrement DNS/eBPF externe n’est produit. Le résultat ne prouve pas l’identité générale de tous les émetteurs webhook, la correction du statut Actions, AgentOS/LLM, ni un gain comparatif. Le parcours de fusion n’a pas été rejoué. Le benchmark apparié reste ouvert faute de runtime/budget fiable autorisé; le produit demeure sans licence choisie.
