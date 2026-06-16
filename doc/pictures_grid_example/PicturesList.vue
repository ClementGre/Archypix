<script lang="ts" setup>

import type {ListPictureData} from "~/types/pictures";

let pictures_store = usePicturesStore()

const click_picture = (e: MouseEvent, picture: ListPictureData) => {
  if (e.ctrlKey || e.metaKey) {
    pictures_store.select_toggle(picture.id)
  } else if (e.shiftKey) {
    pictures_store.select_to(picture.id)
  } else {
    pictures_store.select(picture.id)
  }
}

const query_more = () => {
  pictures_store.query_more()
}

</script>

<template>
  <ul>
    <PictureListElement v-for="data in pictures_store.pictures"
                        :key="data.id"
                        :picture="data"
                        :selected="pictures_store.selected_pictures.includes(data.id)"
                        @click="e => click_picture(e, data)"/>
  </ul>
  <Button v-if="pictures_store.can_query_more" @click="query_more">
    Load more
  </Button>
</template>

<style lang="stylus" scoped>
ul
  overflow scroll
  list-style none
  padding 3px
  margin 0
  display flex
  flex-wrap wrap
  align-content stretch
  gap 3px

  &::after
    content ''
    flex-grow 1000000000


</style>
