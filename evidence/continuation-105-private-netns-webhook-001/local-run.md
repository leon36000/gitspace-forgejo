# CONTINUATION-105 — Forgejo + réconciliation webhook dans un namespace privé

## Périmètre et exécution

Essai local le 26 octobre 2026 avec Bubblewrap 0.9.0 et le binaire `/tmp/forgejo-17.0.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Il annonce Forgejo `17.0.0+gitea-1.22.0`, compilé avec Go 1.26.9; le commit exact de construction du binaire n’est pas établi. Le checkout produit observé est `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` et était modifié.

Commande de lancement :

```sh
python3 scripts/run_forgejo_private_netns_webhook_e2e.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/forgejo-private-netns-webhook-008-state \
  --evidence-dir "$PWD/evidence/continuation-105-private-netns-webhook-001" \
  --timeout-seconds 360
```

Le scénario a créé huit webhooks Forgejo : ouverture/fermeture/modifications d’issue, ouverture/synchronisation/commentaire/fermeture de PR. Les huit tâches ont été livrées au receiver avec HTTP 202 et `is_succeed=true` dans [forgejo-hook-tasks.json](forgejo-hook-tasks.json). La réconciliation par lot a lié quatre événements de PR à une lecture canonique unique (`upstream_read_attempts=1`) et l’état final a confirmé la PR fermée, non fusionnée : [pull-request-batch-reconciliation.json](pull-request-batch-reconciliation.json), [result.json](result.json). Le replay du webhook a reçu 202 sans changer le journal. Le readback de nettoyage confirme l’arrêt des processus et l’absence des listeners suivis.

## Frontière observée

Pendant que le vrai binaire Forgejo écoutait sur `127.0.0.1:3130`, Forgejo et le runner Python appartenaient au namespace réseau `4026535806`, différent du namespace hôte `4026531833`. La connexion depuis l’hôte a reçu `ECONNREFUSED` (`errno=111`); une tentative non privilégiée de rejoindre le namespace a reçu `EPERM`. Le runner et Forgejo partageaient leur namespace isolé, ce qui a permis au gate de faire les lectures requises. Le test de source confirme que le code produit et les métadonnées Git ont été montés en lecture seule sous `/run/workspace` et `/run/source-git`; `/home` restait masqué.

La commande métier s’est terminée avec le code 0 et [result.json](result.json) porte `verdict=passed`. La première enveloppe d’observation, [private-netns-observation.json](private-netns-observation.json), portait à tort `run_status=failed` parce qu’elle cherchait une ligne JSON compacte alors que le runner écrit un objet formaté dans `result.json`. Pour préserver le reçu original, il n’a pas été réécrit. Le readback déterministe [private-netns-reassessment.json](private-netns-reassessment.json) compare ce reçu et le résultat sauvegardé, recalcule le verdict à partir de dix assertions explicites et conserve les SHA-256 des deux sources. Le lecteur corrigé est couvert par `tests.test_forgejo_private_netns_webhook_e2e`.

Le manifeste local append-only vérifie le dernier hash de chaque artefact et conserve les entrées historiques. Les rapports bruts des essais précédents restent distincts dans `continuation-098` à `continuation-104`; ils ne sont pas présentés comme des réussites. L’analyse a trouvé la cause du faux délai : sans option de commentaire, le runner comptait quand même un événement `issue_comment` et attendait neuf livraisons alors que huit étaient attendues. Le comptage est corrigé et couvert par un test; la temporisation métier reste à 20 secondes. Les échecs intermédiaires de montage, d’identification du processus et de lecture du résultat ont aussi été corrigés et conservés.

## Limites

Cette preuve couvre cette machine et cet utilisateur non privilégié. Elle n’exclut pas un writer root/privilégié, une compromission du noyau, ni tous les chemins d’écriture Forgejo. Le scénario ne fusionne pas de PR et ne teste pas CAS ni le credential de fusion. Il complète les preuves egress agent existantes sans répéter leurs sondes API/Web/`receive-pack`. Aucun LLM, AgentOS, coût modèle, service externe, production ou code Forgejo copié/adopté n’est impliqué. La licence et les droits GitSpace restent ouverts; le projet n’est pas complet.
