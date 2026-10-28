# CONTINUATION-399 — suite courante et préflight GitLab PC4

Date : 28 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt était fortement modifié avant ce lot; les modifications existantes ont été conservées.

## Suite hors ligne

Commande :

```text
cargo test --offline --locked --all-targets -- --test-threads=1
```

Le journal complet `cargo-all-targets.log` contient 38 lignes de résumé de cible : 753 tests réussis, 0 échoué, 188 ignorés. Aucun test n’a été ignoré par erreur de compilation. Les scénarios processus explicitement ignorés, dont les E2E Bubblewrap, ne sont pas rejoués par `--all-targets`; les preuves précédentes restent séparées. Cette suite ne démontre pas un gain comparatif ni un parcours LLM.

## Préflight du dépôt GitLab demandé

La cible demandée est `https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git`; le remote local s’appelle `pc4-gitlab`. Le poste PC4 répond au ping Tailscale. Depuis ce worker, le nom MagicDNS n’a pas d’adresse résolue et le probe HTTPS avec l’adresse du pair expire avant TLS; les ports TCP 443 et 80 sont fermés ou filtrés. `git ls-remote` a échoué sur la résolution DNS et n’a lu aucune référence. Aucun commit, stage, push, branche distante ou autre mutation distante n’a été réalisé. Aucun appel HTTP/TLS authentifié n’a atteint GitLab.

Les credentials trouvés dans les courriels n’ont pas été utilisés ni transmis; l’échec de résolution DNS s’est produit avant la connexion. Toutefois, les extraits Gmail affichés par l’outil dans ce fil ont révélé le jeton dédié de l’agent et le mot de passe initial de `root`. Les traiter tous deux comme compromis : révoquer le jeton et changer le mot de passe `root`; aucun secret n’est reproduit ici. Une fois l’URL HTTPS accessible depuis PC6, relire les références sans écriture, vérifier les fichiers locaux à transférer puis préparer une branche de migration séparée, sans force-push ni remplacement de la branche par défaut.

## Limites et état

Cette validation couvre seulement les cibles Cargo exécutables par défaut et un préflight réseau local. Les tests ignorés n’ont pas été exécutés. Aucun accès GitLab authentifié, transfert, merge, AgentOS/LLM, comparaison conventionnelle ou comportement en production n’est démontré. L’état du worktree reste sale à son HEAD antérieur; aucun fichier source n’a été modifié dans ce lot. La mission reste incomplète.

## Note de préservation du manifeste

La première réparation de l’index a utilisé par erreur le chemin relatif `../evidence/SHA256SUMS` depuis le worktree, ce qui a créé le fichier non canonique `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/evidence/SHA256SUMS` avec quatre lignes. Il n’est pas l’index projet utilisé par le vérificateur; les index canoniques `evidence/SHA256SUMS` du worktree et du projet ont ensuite été mis à jour de manière append-only et validés. Le sidecar est laissé en place conformément à la consigne de ne rien supprimer.
