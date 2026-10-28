# WEBHOOK-NATIVE-REPLAY-006 — liaison du corps HTTP brut au task Forgejo

Date : 24 octobre 2026. Run n=1 contre une instance Forgejo v17 locale jetable, avec proxy compteur et receiver sur loopback. Le lot 006 conserve le run historique 005 et ajoute l'assertion manquante signalée par sa revue indépendante.

## Commande du parcours

    PYTHONDONTWRITEBYTECODE=1 python3 scripts/run_forgejo_live_webhook_native_replay_e2e.py \
      --forgejo-binary /tmp/forgejo-17.0.0 \
      --repo-root /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace \
      --state-dir /tmp/forgejo-native-replay-006-state \
      --evidence-dir /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace/evidence/webhook-native-replay-006

Le run compile le receiver Rust avec Cargo hors ligne/verrouillé dans Bubblewrap, démarre Forgejo, le proxy et le receiver en loopback, puis lit les tâches hook_task via SQLite en lecture seule. Aucun credential n'est enregistré dans les preuves.

## Résultat observé

- Forgejo local rapporte 17.0.0+gitea-1.22.0, binaire SHA-256 e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7. La source officielle examinée est le tag v17.0.0, commit 0d791421f85ca13a63b665053d81ea765fbbc571; le commit exact de build du binaire n'est pas établi.
- L'échec initial atteint le proxy en 401; les deux replays manuels atteignent le proxy en 202. Pour chacun des trois POST, le runner lie UUID et statut HTTP à une ligne hook_task, vérifie la longueur, puis compare SHA-256 des octets de corps bruts captés par le proxy à SHA-256 du payload_content Forgejo stocké. Les trois empreintes sont identiques : 5e4d58af1642d0980f8dde4a043394b8963ebd7cd63914a0b8df0f9449dbca1b. Le reçu donne raw_payload_hashes_match=true.
- Après lecture active=false, le replay admin renvoie 303 et crée une tâche, mais celle-ci finit is_delivered=true, is_succeed=false, response_status=0. Aucun quatrième POST n'arrive au proxy et le journal du receiver reste à deux événements. Le statut web 303 n'atteste donc pas la livraison.
- Readback final : hook inactif, processus suivis terminés et listeners absents. L'état privé reste dans /tmp/forgejo-native-replay-006-state.

## Vérifications et provenance

- PYTHONDONTWRITEBYTECODE=1 python3 -m unittest -v tests.test_forgejo_webhook_native_replay_e2e : 8 réussis, 0 échec. Les tests couvrent le binding des hashes bruts au task, et rejettent notamment un hash de requête différent et une requête dupliquée.
- py_compile du runner et du fichier de tests : réussi; les bytecodes de vérification ont été placés sous /tmp.
- Les hashes SHA-256 du runner et du test sont consignés dans provenance.json : 0288e7ede0f1dc462db6e45d6f3c74744919a595035ebb80c8251eae7c0ce4f0 et a3c6221beeae02062cf0e007572230064c1d138ad1004706dd472bb9fb1bfccb. Le journal de compilation isolée, les readbacks, les reçus, le résultat, la provenance et le manifeste sont joints.
- Audit source Forgejo v17 : routers/web/repo/setting/webhook.go:461–480, models/webhook/hooktask.go:153–170, services/webhook/deliver.go:97–105,135–144.

## Limites

Le proxy n'implémente et ne compte que les POST JSON employés par cette configuration. La fenêtre de retry observée dure une seconde; run n=1, compte admin local, lecture d'une table SQLite interne, services sous le même UID. Bubblewrap borne le build, pas ces processus. Le binaire n'est pas relié au commit source examiné. Aucun test Go upstream, agent LLM, AgentOS, Forge Code, RAG ou mémoire n'est exercé ici; ce n'est pas le parcours demande→agent→PR→vérification→fusion et aucune amélioration générale de valeur n'est mesurée.
