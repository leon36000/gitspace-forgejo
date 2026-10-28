# CONTINUATION-014 — refuser au CLI un ancien webhook d’issue mal formé avant toute lecture Forgejo

Date : 2026-10-25. Worktree produit : HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, fortement modifié. Le test et les documents de ce lot ne sont pas commités. Aucun code de production n’a changé dans cette tranche.

## Lacune et scénario

CONTINUATION-012 refusait désormais `review` à l’ingestion et dans la fonction de réconciliation. Les tests existants couvraient les deux contrats; il manquait un test du vrai binaire CLI après replay d’une inbox qui aurait été produite avant ce correctif.

`actual_reconcile_cli_process_rejects_legacy_issue_review_member_before_api_read` utilise d’abord le vrai `forgejo-webhook-local` pour recevoir une issue valide signée. Il modifie ensuite la ligne persistée en y ajoutant `review: null`, forme que l’ancien contrôle d’ingestion, qui ne rejetait que les objets, pouvait accepter. Le test recalcule l’empreinte de la ligne, puis `EventJournal::replay` confirme que la chaîne d’intégrité reste valide. Ce fixture simule un enregistrement historiquement possible; il ne provient pas d’un binaire ancien conservé, et l’ancienne signature HMAC n’est pas présente dans le format journal pour être vérifiée indépendamment.

Le test lance ensuite le vrai `forgejo-reconcile-local` dans le runner Bubblewrap du dépôt, avec `--unshare-net` et `--unshare-pid`, sans fixture Forgejo à l’écoute. Un listener sur le point de terminaison API configuré `127.0.0.1:3130` vérifie directement qu’aucune connexion n’est acceptée. Le CLI doit sortir avec un refus JSON `inconclusive/invalid_envelope`, `upstream_read_attempts=0`, sans instantané canonique; les octets du journal doivent rester inchangés.

## Résultat

Commande :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_rejects_legacy_issue_review_member_before_api_read
```

Résultat final : exit 0; scénario 1/1, 76 autres tests filtrés. Le réseau est isolé par le runner sous Bubblewrap 0.9.0; le binaire CLI, le receiver et le test sont compilés hors ligne/verrouillés avec Rust/Cargo 1.95.0. Voir `runner.log`. `cargo fmt --all -- --check`, `git diff --check` et `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` passent.

Revalidation Cargo : le garde passe 9/9. `cargo metadata --no-deps` confirme `license=null`, `license_file=null`, `publish=[]`, `agent-forge.licensing.decision=no_license_selected`, `redistribution_rights=not_assessed` et `spdx_package_license_declared=NOASSERTION`. Aucun choix de licence n’est fait. Les readbacks sont dans `cargo-license-tests.log`, `cargo-license-verifier.json` et `cargo-metadata-readback.json`.

Versions, révision et hashes des sources dans `tool-versions.txt` et `source-sha256.txt`.

## Portée et limites

Ce test couvre le chemin de replay et de refus du CLI pour un événement `issues` historique avec `review: null`. Les autres formes sont couvertes par les tests de contrat/receiver précédents; elles ne sont pas répétées ici. L’enregistrement historique est simulé avec la formule de hachage actuelle, pas extrait d’une version publiée. Bubblewrap isole le réseau et l’espace PID et monte le code copié en lecture seule; ce n’est pas une séparation multi-tenant ni une preuve que le même UID ne peut jamais lire un secret hôte. Le fixture valide la chaîne de hachage de l’inbox, pas la provenance HMAC historique du contenu modifié.

Aucune instance Forgejo, donnée privée, modèle, AgentOS ou service externe n’est utilisé; aucun coût n’est engagé. Le parcours LLM unique documenté ailleurs n’est ni répété ni une intégration AgentOS ou une comparaison Forgejo+agent+CI. Le résultat valide un refus local précis; il ne mesure pas de gain global. Le projet reste incomplet.
