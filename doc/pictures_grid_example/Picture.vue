<script lang="ts" setup>
import {PictureThumbnail, usePicturesCacheStore} from "~/stores/pictures_cache";
import {decode} from "blurhash";

const props = defineProps(['picture', 'visible', 'loading'])

const emit = defineEmits(['update:loading']);

const picturesCacheStore = usePicturesCacheStore();
const currentPictureId = ref<number | null>(null);

const canvasRef = ref<HTMLCanvasElement | null>(null);
const blurHashLastPictureId = ref<number | null>(null);
const setBlurHashCanvas = () => {
  if (blurHashLastPictureId.value === props.picture?.id) return;
  blurHashLastPictureId.value = props.picture?.id;

  const pixels = decode(props.picture.blurhash, 25, 25);
  const canvas = canvasRef.value;
  if (!canvas) return;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const imageData = ctx.createImageData(25, 25);
  imageData.data.set(pixels);
  ctx.putImageData(imageData, 0, 0);
}

watchEffect(() => {
  const id = props.picture?.id
  if (currentPictureId.value !== null) {
    if (currentPictureId.value === id) {
      return;
    } else {
      picturesCacheStore.releasePictureUrl(currentPictureId.value, PictureThumbnail.Medium);
    }
  }
  if (id && props.loading && props.visible) {
    setBlurHashCanvas();
    picturesCacheStore.getPictureUrl(id, PictureThumbnail.Medium, () => true || props.visible).then((url) => {
      if (!props.visible) {
        picturesCacheStore.releasePictureUrl(id, PictureThumbnail.Medium);
      } else {
        emit('update:loading', false);
        currentPictureId.value = id;
        thumbStyle["background-image"] = `url('${url}')`;
      }
    })
  }
})


const thumbStyle = reactive({
  'aspect-ratio': '1/1',
  'background-image': '',
})

watch(props, () => {
  let h = 140;
  let w = h * props.picture.width / props.picture.height;
  thumbStyle["aspect-ratio"] = w + '/' + h

  if (!props.visible && currentPictureId.value !== null) {
    emit('update:loading', true);
    picturesCacheStore.releasePictureUrl(currentPictureId.value, PictureThumbnail.Medium);
    currentPictureId.value = null;
    thumbStyle["background-image"] = ``;
  }
}, {immediate: true})

onUnmounted(() => {
  if (currentPictureId.value !== null) {
    picturesCacheStore.releasePictureUrl(currentPictureId.value, PictureThumbnail.Medium);
  }
});

</script>

<template>
  <Toast/>
  <div :style="thumbStyle" class="thumb rounded-md drop-shadow-sm">
    <template v-if="loading">
      <canvas
          v-if="props.picture.blurhash"
          ref="canvasRef"
          :height="25"
          :width="25"
          class="w-full h-full rounded-xs"
      ></canvas>
      <Skeleton v-else border-radius="2px" height="100%" width="100%"></Skeleton>
    </template>
  </div>
</template>

<style lang="stylus" scoped>
.thumb {
  width: 100%;
  height: 100%;
  border-radius: 2px;

  background-repeat: no-repeat;
  background-size: cover;
  background-position: center;
}
</style>
