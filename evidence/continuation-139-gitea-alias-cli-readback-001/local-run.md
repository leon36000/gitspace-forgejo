# CONTINUATION-139 — alias Gitea seuls jusqu’au readback du CLI

Date : 26 octobre 2026 (America/Montreal). Résultat : **réussi**, un scénario isolé (`n=1`).

## Lacune et scénario

CONTINUATION-138 prouvait que le vrai processus `forgejo-webhook-local` accepte une livraison qui contient uniquement les trois noms d’en-tête `X-Gitea-*`, mais le test s’arrêtait après l’append durable. Ce scénario relie maintenant cette même variante au binaire réel `forgejo-reconcile-local`.

Le test signé synthétique décrit une issue #41 fermée. Il remplace les trois en-têtes Forgejo par leurs alias Gitea, vérifie qu’aucun en-tête Forgejo ne subsiste, envoie la requête au vrai receiver, arrête celui-ci et relit le journal. Le record garde l’ID `gitea-alias-cli-41` et le type canonique `forgejo.webhook.v1.issues`. Le vrai CLI reçoit seulement le chemin du journal et l’ID, sans token ni en-tête `Authorization`; l’API loopback de test répond que l’issue est actuellement `open`.

## Observations

- Receiver : HTTP 202, `accepted=true`, `duplicate=false`, séquence 1, `reconcile_required=true`.
- CLI : `verdict=current_state_read`, ressource `Issue/41`, URL canonique attendue et un seul `GET /api/v1/repos/forge-admin/agent-forge-core/issues/41` observé par le fixture.
- La réponse du CLI vient de l’état courant `open`, pas du payload obsolète `closed`.
- Le journal est comparé octet pour octet avant/après réconciliation et demeure identique.
- Le test processus ciblé passe 1/1, 116 filtrés. Le runner construit depuis une copie de source, Cargo verrouillé/hors ligne, puis exécute le test sous Bubblewrap avec `--unshare-net`/`--unshare-pid`; le fixture loopback est la seule destination API dans l’espace réseau isolé.

## Commandes et vérifications

Commande du scénario :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_reads_gitea_alias_only_delivery
```

`cargo fmt --all -- --check`, `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` et `git diff --check` passent sur l’état final. Les premiers contrôles rustfmt ont relevé des mises en forme locales du nouveau test; les corrections et toutes les sorties intermédiaires sont gardées dans ce dossier, et `rustfmt-final3.log` est vert. Aucun code de production Rust n’a changé; les modifications concernent un test de processus et sa liste d’exécution isolée, ainsi que la documentation.

## Portée et limites

Le corps et le HMAC sont synthétiques; l’API de lecture est une fixture loopback, pas une instance Forgejo ni Gitea. Cela ne démontre ni une livraison produite par Gitea, ni toutes les variantes d’en-têtes mixtes/dupliqués, ni le comportement de proxys qui normalisent les noms. Aucune identité ou donnée privée, dépense, production, LLM, AgentOS, RAG, mémoire, vérificateur indépendant ou fusion n’est impliqué. La mission et le benchmark comparatif restent incomplets.

## Révision et sources locales

HEAD de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`; le worktree est fortement modifié et ce lot n’est pas commité. `source-hashes.txt` lie le test, le runner, le checkpoint et la matrice à leurs empreintes prises pour ce reçu. La version des outils et l’instant UTC sont dans `tool-versions.txt`; les commandes et le résultat complet du runner sont archivés.
