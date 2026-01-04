<script lang="ts">
    let searchQuery = ""
</script>

<div id="search-form" class="search" class:active={searchQuery !== ""}>
    <form action="#" method="GET">
        <input
            bind:value={searchQuery}
            type="text"
            name="q"
            placeholder="Введите команду или скажите «Джарвис» ..."
            autocomplete="off"
            minlength="3"
            maxlength="30"
        />
        <button type="submit" aria-label="Search"></button>
        <small>Enter</small>
    </form>
</div>
