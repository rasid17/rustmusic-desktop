<script lang="ts">
  import Icon from "@iconify/svelte";
  import { popinStore } from "#lib/stores/ui/popin.store";
  import PopinFooter from "#lib/components/ui/popin/PopinFooter.svelte";
  import { t } from "#lib/i18n";

  let {
    message = "",
    confirmText = undefined,
    cancelText = undefined,
    onConfirm = () => {},
    onCancel = () => {},
    variant = "primary"
  }: {
    message?: string;
    confirmText?: string;
    cancelText?: string;
    onConfirm?: (() => void) | (() => Promise<void>);
    onCancel?: (() => void) | (() => Promise<void>);
    variant?: "danger" | "primary";
  } = $props();

  const resolvedConfirmText = $derived(confirmText ?? $t("common.confirm"));
  const resolvedCancelText = $derived(cancelText ?? $t("common.cancel"));

  let isLoading = $state(false);
  const danger = $derived(variant === "danger");

  async function handleConfirm() {
    isLoading = true;
    try {
      await onConfirm();
      popinStore.close();
    } catch (error) {
      console.error("Erreur lors de la confirmation:", error);
    } finally {
      isLoading = false;
    }
  }

  async function handleCancel() {
    isLoading = true;
    try {
      await onCancel();
      popinStore.close();
    } catch (error) {
      console.error("Erreur lors de l'annulation:", error);
    } finally {
      isLoading = false;
    }
  }
</script>

<!-- Message, précédé d'une pastille teintée selon la gravité -->
<div class="flex items-start gap-3.5">
  <span
    class="shrink-0 w-10 h-10 rounded-xl flex items-center justify-center border
           {danger
             ? 'bg-red-500/8 border-red-500/25 text-red-600 dark:text-red-400'
             : 'bg-(--rg-gbg) border-(--rg-gbd) text-(--rg-g)'}"
  >
    <Icon icon={danger ? "material-symbols:warning-outline-rounded" : "material-symbols:help-outline-rounded"} width="20" />
  </span>
  <p class="pt-2 text-sm leading-relaxed text-(--rg-tx2) text-pretty">
    {message}
  </p>
</div>

<PopinFooter
  bleed
  cancelLabel={resolvedCancelText}
  oncancel={handleCancel}
  cancelDisabled={isLoading}
  submitLabel={resolvedConfirmText}
  submitIcon={danger ? "material-symbols:delete-outline-rounded" : "material-symbols:check-rounded"}
  onsubmit={handleConfirm}
  busy={isLoading}
  tone={danger ? "danger" : "accent"}
/>
