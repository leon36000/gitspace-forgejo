# CONTINUATION-428 — Forgejo 17.0.0 : webhook natif Release puis PR

Date : 28 octobre 2026 (America/Montreal). Exécution PC6 locale isolée; source PC6 au HEAD f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0 avec worktree modifié. Aucun agent LLM, AgentOS, RAG ou mémoire n’a participé.

## Échec actionnable et correction

Le premier parcours natif est consigné dans le reçu d’échec ../forgejo-native-release-webhook-001/failure.json : échec au stade deliver_opened_pull_request_event, ValueError: journal record is not the expected pull-request event. L’instance avait déjà émis quatre événements d’issue puis le Release published en séquence 5; le code attendait l’index PR d’un scénario sans Release et lisait donc le Release comme un événement PR. Aucun verdict réussi n’a été déduit du run échoué.

Le runner calcule maintenant l’index PR après les événements d’issue, la réouverture éventuelle et le Release éventuel; il dérive les attentes de synchronisation et de commentaire du nombre de records après ouverture PR. La suite de contrats complète, après la correction, passe 190/190. Les trois contrôles CargoTmpfsBuildStagingTests couvrent staging privé, refus d’un filesystem autre que tmpfs et refus des liens symboliques; le contrôle CLI confirme --build-staging-root.

## Parcours natif réussi

Le runner utilise l’image locale codeberg.org/forgejo/forgejo:17.0.0 (ID sha256:06d38310de496a10186d952bdad5ca764c153c8023ede8dfa0944c79a1dc5412, linux/arm64). Dans l’image, /usr/local/bin/forgejo est un wrapper et /app/gitea/gitea le binaire ELF statique; version lue directement sur PC6 : forgejo version 17.0.0+gitea-1.22.0, SHA-256 e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7.

Commande principale : scripts/run_forgejo_live_webhook_reconcile_emitted.py --forgejo-binary <binaire-extrait> --repo-root <worktree> --state-dir /tmp/forgejo-native-release-002-state --evidence-dir evidence/forgejo-native-release-webhook-002 --release-webhook --build-staging-root /dev/shm. L’extraction Docker est faite depuis l’image locale avec un conteneur --network none. Forgejo et le receiver s’exécutent ensuite comme processus locaux sur loopback; Cargo est --offline --locked dans Bubblewrap sans réseau. Le staging hôte confirme filesystem_type=tmpfs, répertoire temporaire privé, retiré après compilation (provenance.json). Le build a utilisé rustc 1.95.0 et Cargo 1.95.0.

Le résultat du harnais est passed; result.json, provenance.json, les reçus de réconciliation, duplicate-delivery-replay.json, invalid-credential-result.json et port-cleanup-readback.json sont contrôlés par SHA-256. Release published : séquence 5, lecture actuelle Forgejo present. PR : ouverture séquence 6, commentaire de conversation séquence 7, fermeture séquence 8; l’API confirme closed, merged=false. La réconciliation tardive de l’événement PR ouvert observe l’état courant fermé, pas un ancien état présenté comme actuel.

Le receiver est tué par SIGKILL puis redémarré avec le même journal; la rediffusion Release répond HTTP 202 duplicate=true et conserve 8 records, le même nombre d’octets et la même empreinte du journal. Le credential invalide est refusé (upstream_unauthorized, exit 1, verdict inconclusive). Le webhook est désactivé puis relu; Forgejo et receiver ont terminé et aucun des deux ports de loopback ne reste en écoute.

Le manifeste local conserve plusieurs empreintes historiques de `commands.txt` après l’ajout append-only du journal de vérification. `sha256sum -c` signale ces anciennes lignes; `scripts/verify_latest_sha256_manifest.py --root evidence/forgejo-native-release-webhook-002 evidence/forgejo-native-release-webhook-002/SHA256SUMS` passe pour les 22 chemins uniques sans divergence.

## Portée et limites

C’est un seul run natif de webhook contre une instance Forgejo jetable; la PR est fermée sans merge contrôlé. Le premier échec suivi d’une correction est prouvé, mais aucun agent LLM n’a produit le changement, et ceci ne démontre ni la livraison complète avec agent ni un gain de succès, temps, coût ou tokens. Le run ne simule pas une CI GitLab. Le staging et le runtime restent PC6; les clés/données d’état privées Forgejo restent sur PC6 sous /tmp et ne sont pas synchronisées. L’évidence ne contient pas le secret webhook.
