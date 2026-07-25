<script lang="ts">
	import { Select as SelectPrimitive } from "bits-ui";

	let { placeholder, ...restProps }: { placeholder?: string } & Record<string, unknown> = $props();
</script>

<SelectPrimitive.Value {placeholder} {...restProps} />
