# CONTINUATION-429 — composition webhook natif Release + labels — 28 octobre 2026

## Portée

Un parcours Forgejo local jetable combine `release.published`, les transitions de labels d’une issue et d’une pull request, les événements d’issue/PR/commentaire du scénario de base et leur réconciliation. La combinaison exacte Release + labels issue + labels PR n’apparaît pas dans les reçus natifs antérieurs parcourus avant ce run. Le runner et le runtime produit n’ont pas été modifiés dans ce lot.

Source au HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, worktree déjà modifié. L’image locale `codeberg.org/forgejo/forgejo:17.0.0` a l’ID `sha256:06d38310de496a10186d952bdad5ca764c153c8023ede8dfa0944c79a1dc5412` (`linux/arm64`). Le binaire `/app/gitea/gitea` annonce Forgejo `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le commit Go exact correspondant à cette image n’est pas établi par ce reçu.

## Commande et résultat

```text
python3 -B scripts/run_forgejo_live_webhook_reconcile_emitted.py \
  --forgejo-binary /tmp/forgejo-native-composed-429-bin \
  --repo-root /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace \
  --state-dir /tmp/forgejo-native-composed-429-state \
  --evidence-dir evidence/continuation-429-forgejo-release-label-composition-001 \
  --release-webhook --change-issue-labels --change-pull-request-labels \
  --build-staging-root /dev/shm
```

Résultat `passed`; Forgejo a émis et le receiver a accepté **12/12** tâches par HTTP 202. Ordre journalisé : issue ouverte (1), label issue ajouté puis effacé (2–3), issue fermée et éditée deux fois (4–6), Release publiée (7), PR ouverte et commentaire créé (8–9), label PR ajouté puis effacé (10–11), PR fermée sans fusion (12). Les readbacks API après retrait de chaque label sont vides; la projection décrit l’état courant de l’issue et de la PR comme fermés. Les payloads de labels restent des notifications et ne remplacent pas l’API courante.

Le receiver a été tué par SIGKILL puis relancé avec le même journal. Le rejeu de la livraison Release répond `202 duplicate=true`; les **12** lignes, le nombre d’octets et le SHA-256 du journal restent identiques (`20dba1c3100033103ceb8a94f6f5282ca09e8f5c672ac4a8daf4248b9ad2e40f`). Le credential volontairement invalide reçoit `upstream_unauthorized` et un verdict `inconclusive`. Le hook est désactivé et relu; les processus et listeners suivis sont absents après arrêt.

Le build du récepteur et du reconciler a utilisé Cargo/Rust `1.95.0`, Bubblewrap `0.9.0`, staging vérifié sur `tmpfs`, namespaces réseau/PID isolés, source et registre en lecture seule, et Cargo `--offline --locked`. Hashes des exécutables et des sources compilées : `provenance.json`.

Vérifications fraîches : `python3 -B tests/test_forgejo_emitted_webhook_e2e.py` (**190/190**); garde Cargo et ses tests (**12/12**); `git diff --check` (code 0); vérificateurs des manifestes local et global (toutes les dernières empreintes valides, aucun chemin absent ou divergent).

Le garde Cargo confirme sans attribuer de licence : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`, `NOASSERTION`. Le champ SPDX descriptif ne constitue pas un choix de licence ni une autorisation de redistribution.

## Limites

Un seul run dans une instance Forgejo jetable locale (`n=1`). Cela exerce la composition des événements, la lecture de l’état courant et le rejeu du receiver; ce n’est pas un parcours avec agent, une PR fusionnée, une vérification post-fusion, une CI GitLab ou un benchmark comparatif. Aucun LLM, AgentOS, RAG ou moteur de mémoire n’a été utilisé. L’image identifie une version et un hash de binaire, pas le commit amont Go exact. Ce run ne prouve pas un gain général ni la couverture de tous les ordres d’événements.
