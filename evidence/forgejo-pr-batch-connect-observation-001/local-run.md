# FORGEJO-PR-BATCH-CONNECT-OBSERVATION-001 — observation locale de connexion batch

Date : 25 octobre 2026. Worktree : `local/WEBHOOK-005-canonical-links`, HEAD observé `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; arbre déjà fortement modifié. Aucun commit ni staging effectué. Voir `provenance.json`, `source-sha256-final.txt` et `SHA256SUMS.final` pour les empreintes.

## Résultat

Le scénario local minimal émet sept tâches webhook Forgejo, toutes acceptées HTTP 202. Trois deliveries PR (`opened`, commentaire de conversation, `closed`) sont liées à la PR #2, puis réconciliées en batch : trois résultats ordonnés et liés, `verdict=current_state_read`, PR fermée non fusionnée. Le CLI indique `upstream_read_attempts=1`.

Un `strace 6.8` indépendant du compteur applicatif a suivi uniquement les appels système `connect` du processus batch et de ses enfants. Il a relevé une tentative vers `127.0.0.1:3130`, cohérente avec le résultat API réussi; le syscall non bloquant retourne `EINPROGRESS` avant que curl ne reçoive la réponse. Aucun autre connect réseau n’a été observé. Les appels de socket AF_UNIX éventuels sont comptés séparément; aucun argument, en-tête HTTP ni corps n’est tracé. La sortie contient aussi un événement `SIGCHLD` standard de strace. Il s’agit d’une observation syscall locale, pas d’une capture indépendante de la requête HTTP ou de sa réponse.

Commande :

```text
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py --forgejo-binary /tmp/forgejo-17.0.0 --state-dir /tmp/forgejo-pr-batch-connect-observation-001-state --evidence-dir evidence/forgejo-pr-batch-connect-observation-001 --batch-pull-request-events --observe-batch-network
```

Le récepteur a été tué par SIGKILL puis relancé avec le même journal; le rejeu n’en a changé ni le nombre ni les octets. L’API de l’instance confirme la PR fermée/non fusionnée. La hook est désactivée par readback; Forgejo et le récepteur sont arrêtés et les ports du fixture n’ont plus de listener.

## Vérifications et provenance

- Driver : 75/75 tests Python; `py_compile`, vérification Cargo de l’état de licence 7/7 et `git diff --check` passent.
- `Cargo.toml` demeure sans `license` ni `license-file`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed`; aucune licence n’a été choisie.
- Le binaire Forgejo local est v17.0.0 (`e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`). Le source officiel examiné est au commit `0d791421f85ca13a63b665053d81ea765fbbc571`; le commit exact de build du binaire n’est pas établi.
- Le CLI Rust a été rebâti offline/locked avec Cargo/rustc 1.95.0 sous Bubblewrap 0.9.0, réseau et PID namespace isolés pour le build. Aucun code Rust n’a changé dans ce lot; la suite Rust complète n’a pas été relancée.
- Le fichier `batch-connect-trace.log` ne contient que les résultats de `connect` et l’événement de sortie de l’enfant. Le runner a vérifié avant archivage que les credentials générés ne figurent pas dans cette sortie.

## Portée et limites

Un run (`n=1`) sur Forgejo jetable vérifie la corrélation d’une lecture batch avec une seule connexion locale; il ne démontre pas indépendamment la méthode HTTP, le chemin, le statut de réponse ni le nombre de lectures sur le serveur. Le compteur CLI reste une télémétrie applicative. Aucun LLM, AgentOS/RAG/mémoire, CI conventionnelle, service externe ou benchmark comparatif n’a été utilisé. Forgejo, le récepteur et le CLI tournent sous le même compte hôte; aucun code hostile n’est exécuté. Aucun code Forgejo/GitSpace n’est copié et la licence produit reste non choisie.

Suite utile : reprendre le protocole LFS/fence et tester son témoin durable dans le scratch séparé, ou ajouter une observation HTTP loopback dédiée si elle peut rester plus simple que modifier l’origine fixe autorisée par le reconciler. Ne pas annoncer une capture HTTP ni un gain de productivité à partir de cette mesure syscall.
