# CONTINUATION-390 — action d’erreur exploitable pour `comment.id`

Date : 28 octobre 2026. Branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Défaut et correction

Le receiver rejetait correctement `comment.id=0` et exposait `details.field_path=payload.comment.id`, mais renvoyait l’action générique destinée aux champs non spécialisés. Le test de processus a échoué d’abord sur cette action, avant la correction runtime; la sortie rouge est conservée dans `receiver-comment-id-red.log`.

Le mapping d’erreur HTTP possède maintenant une branche `comment.id` qui demande un entier JSON positif, indique l’endpoint de capacités et permet la redelivery avec le même delivery ID. Il rappelle que le corps refusé n’a pas été journalisé. Les règles d’admission et le format des événements acceptés restent inchangés.

## Résultats

Le test du vrai `forgejo-webhook-local` passe sous Bubblewrap `--unshare-net --unshare-pid` : HTTP 400, `event_payload_invalid`, `retryable=false`, chemin exact, action exacte et inbox vide. Résultat 1/1, 184 filtrés. `cargo fmt --all -- --check` et `git diff --check` passent. Les commandes et sorties rouge/verte sont jointes dans `verification-commands.txt` et les fichiers de log.

## Limites

Le test couvre `comment.id=0`; une chaîne, un nombre négatif, un flottant, un overflow et l’absence du champ ne sont pas rejoués dans ce processus. Les contraintes de parsing sont testées au niveau réconciliateur. Aucun événement Forgejo live, AgentOS/LLM, PR/merge ou benchmark n’est impliqué. Aucun accès externe ni changement Cargo n’a eu lieu.
