# WEBHOOK-TORN-TAIL-PROCESS-RECOVERY-001 — récupération du journal au démarrage du receiver

Date : 26 octobre 2026. Exécution locale dans le worktree `webhook-005-workspace`, basé sur `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt était déjà fortement modifié. Ce lot ajoute une couverture d’intégration dans `tests/webhook_receiver_process_e2e.rs`; il ne change pas la logique de production du journal ou du receiver. Aucun service externe, Forgejo live, LLM, secret réel, commit ou changement de production n’a été utilisé.

## Lacune ciblée

Le test unitaire `journal_deduplicates_replays_and_serializes_concurrent_events` dans `src/lib.rs` avait déjà vérifié qu’un journal générique peut ignorer un suffixe final incomplet, puis accepter un append suivant. Le code de `forgejo-webhook-local::run` appelle `EventJournal::replay()` avant de lier le port et de démarrer le serveur. Il manquait un scénario processus intégrant cette récupération de démarrage avec une livraison webhook signée, son rejeu après réparation et la réconciliation.

## Scénario et résultat

Le nouveau test `actual_receiver_repairs_a_torn_webhook_tail_before_redelivery_and_reconcile` démarre le vrai binaire du receiver local, accepte une livraison `issues/opened`, puis l’arrête proprement. Il ajoute au fichier de journal un préfixe JSON volontairement incomplet après la dernière ligne valide et appelle `sync_all` pour conserver le fixture de panne. Au redémarrage, la sonde de disponibilité confirme que le receiver a terminé son replay et est prêt; les octets du journal sont alors exactement ceux de la ligne complète précédente, donc seul le suffixe incomplet a été écarté.

Le test envoie ensuite une seconde livraison signée. Le receiver répond `202`, `accepted=true`, `duplicate=false`, `sequence=2`; le rejeu identique répond `202`, `duplicate=true`. Après arrêt, le journal relu contient les deux livraisons et `reconcile_inbox_events` retourne l’état courant de fixture pour chacune. Le run ciblé a réussi : 1 test réussi, 0 échec, 92 filtrés.

## Vérifications et portée

- La commande exacte et ses versions sont consignées dans `commands.txt` et `toolchain.txt`; la sortie est dans `focused-test.log` avec code de sortie 0.
- `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` passent.
- L’état Cargo relu reste `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed`, marqueur personnalisé `NOASSERTION`.
- Les hashes du code examiné sont dans `source-hashes.txt`; les manifestes locaux et append-only sont vérifiés par `update_manifests.py`.

Il s’agit d’un fixture local déterministe, n=1. Le suffixe incomplet est synthétisé par le test, pas produit par une panne électrique ou un crash noyau pendant une vraie écriture partielle. La livraison de reprise est renvoyée par le client de test, pas par une politique de retry native Forgejo. Ce test ne prouve ni la durabilité sur d’autres systèmes de fichiers, ni le traitement d’une ligne complète corrompue, ni un gain comparatif. Il complète le test distinct CONTINUATION-065 (SIGKILL réel après append durable avant ACK); il ne le rejoue pas.

Aucun appel de modèle ou coût externe n’a eu lieu. AgentOS/LLM reste non démontré, les droits GitSpace et la licence produit restent non résolus, et le benchmark apparié reste ouvert. La mission n’est pas complète.
