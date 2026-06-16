<script lang="ts" setup>
import {useElementVisibility} from '@vueuse/core'
import type {ListPictureData} from "~/types/pictures";

const props = defineProps<{
  picture: ListPictureData,
  selected: boolean,
}>()

const loading = ref(true)

const target = useTemplateRef<HTMLImageElement | null>('target')
const targetIsVisible = useElementVisibility(target, {
  rootMargin: '0px 0px 0px 0px',
})

let h = 140;
let w = h * props.picture.width / props.picture.height;
const liStyle = reactive({
  'flex-basis': w + 'px',
  'flex-grow': w,
})

</script>

<template>
  <li ref="target" :class="{thumb: !loading, loading: loading, selected: props.selected, 'bg-slate-200': true}" :style="liStyle">
    <Picture v-model:loading="loading" :picture="props.picture" :visible="targetIsVisible"/>
    <div v-if="props.selected" class="selected-overlay"/>
  </li>
</template>

<style lang="stylus" scoped>

li {
  flex-grow: 1;
  animation: fadeIn .2s;
  position: relative;
  background none

  .selected-overlay {
    position: absolute;
    top: -1px;
    left: -1px;
    width: calc(100% + 2px);
    height: calc(100% + 2px);
    border-radius: 3px;
    border: 3px solid var(--p-teal-300);
  }

  .thumb {
    width: 100%;
    height: 100%;
    border-radius: 2px;

    background-repeat: no-repeat;
    background-size: cover;
    background-position: center;
  }

}
</style>
