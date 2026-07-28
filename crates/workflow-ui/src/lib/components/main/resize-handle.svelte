<script lang="ts">
	let {
		onResize,
	}: {
		onResize: (deltaX: number) => void;
	} = $props();

	let rafId: number | null = null;

	function startResize(e: MouseEvent) {
		e.preventDefault();
		const startX = e.clientX;
		function onMove(ev: MouseEvent) {
			if (rafId != null) cancelAnimationFrame(rafId);
			rafId = requestAnimationFrame(() => {
				rafId = null;
				onResize(startX - ev.clientX);
			});
		}
		function onUp() {
			if (rafId != null) {
				cancelAnimationFrame(rafId);
				rafId = null;
			}
			document.removeEventListener("mousemove", onMove);
			document.removeEventListener("mouseup", onUp);
			document.body.style.cursor = "";
			document.body.style.userSelect = "";
		}
		document.addEventListener("mousemove", onMove);
		document.addEventListener("mouseup", onUp);
		document.body.style.cursor = "col-resize";
		document.body.style.userSelect = "none";
	}
</script>

<div
	role="presentation"
	class="absolute inset-y-0 -left-[3px] w-[7px] z-10 cursor-col-resize group"
	onmousedown={startResize}
>
	<div
		class="absolute inset-y-0 left-1/2 -translate-x-1/2 w-px bg-border group-hover:bg-accent-foreground/20 group-active:bg-accent-foreground/30 transition-colors"
	></div>
	<div
		class="absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 flex flex-col gap-[3px] opacity-0 group-hover:opacity-100 transition-opacity"
	>
		<div class="size-[3px] rounded-full bg-accent-foreground/30"></div>
		<div class="size-[3px] rounded-full bg-accent-foreground/30"></div>
		<div class="size-[3px] rounded-full bg-accent-foreground/30"></div>
	</div>
</div>
