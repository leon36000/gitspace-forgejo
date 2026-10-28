# WEBHOOK-RETRY-RESUME-MIXED-BATCH-001 — reprendre un lot après un échec transitoire

Date : 25 octobre 2026. `HEAD` observé avant la tranche : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le checkout est fortement modifié. Les empreintes des sources courantes et des fichiers de preuve figurent dans `source-sha256.txt` et `SHA256SUMS`.

## Parcours et résultat

Le test `actual_reconcile_cli_process_resumes_mixed_batch_after_transient_failure` utilise le vrai receiver et le vrai binaire `forgejo-reconcile-local`, avec deux événements webhook HMAC signés ajoutés au journal durable : issue 41, puis issue 42. La fixture HTTP loopback renvoie trois HTTP 503 pour la première issue, puis `200/open` pour la seconde. Le premier processus effectue quatre GET dans cet ordre, termine avec code 1 et produit un JSON versionné de verdict batch `inconclusive`. La ligne d’issue 41 porte `upstream_http_status`, `http_status=503`, `retryable=true`, sans état canonique; l’issue 42 porte `current_state_read/open`. Un corps d’erreur synthétique est absent de stdout/stderr.

Après la sortie du premier processus, le test confirme que le fichier inbox est identique octet pour octet à son instantané précédant la réconciliation. Un nouveau processus lit les mêmes deux IDs depuis le même journal : la fixture renvoie `200/open` pour les issues 41 et 42, dans cet ordre. Les deux lignes deviennent `current_state_read/open`, le verdict du batch est `current_state_read`, le processus termine avec succès et l’inbox reste encore identique.

## Vérification, périmètre et limites

Commande ciblée : `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_resumes_mixed_batch_after_transient_failure`. Le runner compile hors ligne et avec lockfile une copie des sources puis exécute le test sous Bubblewrap avec les namespaces réseau et PID isolés, un home temporaire et des limites `prlimit`. Résultat final : `1 passed; 0 failed; 0 ignored; 44 filtered out`. Rustfmt, `git diff --check` et la garde d’état de licence Cargo passent également.

Cette tranche ajoute un test processus et son entrée dans le runner; aucune logique runtime n’a changé. Les deux processus et l’API sont locaux au test : cela ne démontre pas le comportement contre une instance Forgejo réelle, un crash du système, une livraison de webhook après redémarrage de la machine, la conservation durable au-delà de l’inbox locale ni une politique de backoff temporel avec `Retry-After`. L’erreur 503 est construite par une fixture et ne représente pas un incident de production. Le test n’utilise ni modèle, ni AgentOS, ni secret réel, ni réseau externe. La licence Cargo demeure non choisie (`license=null`, `license_file=null`, `publish=[]`), avec droits de redistribution `not_assessed`. Aucun gain comparatif n’est établi; le parcours avec LLM et la comparaison conventionnelle contrôlée restent à faire.

