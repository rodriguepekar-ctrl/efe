# Glossaire

| Terme | Définition |
|-------|------------|
| **EFE** | Essaim Fluide Éphémère (Ephemeral Fluid Swarm). |
| **ELS** | Espace Latent Partagé. Tenseur `[N, D]` en HBM où vivent les nano-agents. |
| **Nano-agent (NA)** | Agent éphémère (1–50 ms) spécialisé, position dans l'ELS. |
| **Template** | Classe de spécialisation d'un NA (MVP : SEM, LOG, CRIT). |
| **Round** | Itération de fusion (Swarm Attention + Langevin). MVP : 1 à 8. |
| **Consensus** | État où la variance intra-essaim et le gradient sont sous seuils. |
| **Fréchet mean** | Moyenne géodésique sur manifold Riemannien. |
| **Dissolution** | Phase finale : extraction centroïde, décodage, libération des NA. |
| **Cristallisation** | Distillation d'un pattern d'essaim en un NA monolithique (P3+). |
| **Ancre** | Vecteur prior permanent qui guide la projection d'alignement. |
| **Salience** | Poids d'un NA dans la fusion (entre 0 et 1). |
| **Mode collapse** | Situation où tous les NA convergent vers le même point trop tôt. |
| **Arena HBM** | Région mémoire GPU allouée linéairement, reset en fin de requête. |
| **Langevin SDE** | Dynamique bruitée `dh = -η∇U dt + σ dW`. |
| **Swarm Attention** | Variante d'attention dépendant du template de chaque agent. |
| **Breach** | Dépassement du seuil d'alignement lors de la projection finale. |
| **Constitution** | Objet immuable contenant les ancres et zones interdites. |
| **HBM** | High Bandwidth Memory (mémoire GPU). |
| **Triton** | DSL Python pour écrire des kernels GPU. |
| **Teacher** | Modèle de fondation utilisé pour la distillation (ex. Llama 3 8B). |
