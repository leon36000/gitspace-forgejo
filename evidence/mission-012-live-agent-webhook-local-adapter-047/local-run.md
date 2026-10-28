# Mission 012 composé 047 — refus après avance concurrente de `main`

Date : 25 octobre 2026. Branche produit `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, worktree fortement modifié. La composition a capturé 83 entrées (2 579 559 octets), avec le même snapshot de sources que 046 : archive SHA-256 `b42217acd460876f947130d3486a750792091e52ae02294451522268660939b2`, source-map SHA-256 `da86219a889ec649cb6298d41bcb80552150f003ec72cd575dc7523f7d27103c`.

Commande exécutée :

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --state-dir /tmp/mission012-composed-adapter-047 \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --evidence-dir evidence/mission-012-live-agent-webhook-local-adapter-047 \
  --execution-id live-mcp-047 \
  --allocation-id mission012-webhook-live-047 \
  --budget-cap-usd 0.01 \
  --agent-adapter local-test-adapter \
  --inject-concurrent-base-advance
```

Le composeur a quitté avec code 0 et son verdict est `refused_after_concurrent_base_advance_before_merge`. Le base admis était `5b9cb2c24ae7075677df51eb06353c3651191447`; l’exécutant `local-test-adapter` a proposé une modification uniquement de `src/lib.rs`, commit `3d668bfd7042750c8c0cacbbe80338914fdc6247` et SHA de source `173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15`. Comme pour 046, aucun modèle n’a été invoqué, aucun token modèle n’a été consommé et le coût déclaré de l’adaptateur est nul. La tâche passe par le même échec initial actionnable (`left: -2`, attendu `10`), le candidat passe les deux tests, et le vérificateur indépendant revérifie le candidat avant la tentative de merge.

Le scénario a créé une seconde PR approuvée qui a modifié `README.md` puis avancé `main` de `5b9cb2c24ae7075677df51eb06353c3651191447` à `0dc77692f86e9cba3a6b08be3b9f14aaee18b151`. L’autorité de fusion a refusé la PR cible #2 avec `invalid_review_base_binding`; `merge_attempted=false`, `merge_authorized=false`, `main` reste sur le commit concurrent, et la PR cible demeure ouverte/non fusionnée. La PR concurrente #3, autorisée par la fixture, est fusionnée avec HTTP 200. Ce test montre le refus d’une preuve d’approbation liée à une base devenue obsolète pour le chemin exercé. Il ne démontre pas l’exclusion atomique d’un writer externe ne suivant pas le lock coopératif : le reçu le déclare explicitement absent.

L’instance Forgejo v17 est privée et temporaire à `127.0.0.1:3132`, isolée sous `/tmp/mission012-composed-adapter-047`; le runner vérifie aussi les refus d’identifiants et l’admission webhook. Le binaire officiel signé et son hash sont consignés dans [la provenance 046](../mission-012-live-agent-webhook-local-adapter-046/forgejo-binary-provenance.json). Le manifeste de ce dossier couvre le résultat, les deux PR, les preuves du vérifieur et les reçus d’admission.

Cette exécution complète le parcours concurrent sur les sources actuelles, mais reste un adaptateur déterministe sans LLM. Un seul cas de base périmée ne démontre ni sécurité générale contre writers externes, ni gain par rapport à Forgejo + agent + CI conventionnels. Les sorties, readbacks et manifestes du run sont dans ce dossier.
