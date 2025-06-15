import { Client, TextChannel, EmbedBuilder } from "discord.js";
import notificationConfig from "../../models/notificationConfig";
import RSSParser from "rss-parser";
import config from "../../config.json";

const parser = new RSSParser();

// 檢查 YouTube 影片的主要函數
async function checkYoutubeVideos(client: Client) {
  try {
    console.log("🔍 開始檢查 YouTube 影片更新...");

    // 獲取所有啟用的通知設定
    const notifications = await notificationConfig.find({ isActive: true });

    if (notifications.length === 0) {
      console.log("📝 目前沒有啟用的通知設定");
      return;
    }

    console.log(`📋 找到 ${notifications.length} 個通知設定`);

    // 處理每個通知設定
    for (const notification of notifications) {
      try {
        await processNotification(client, notification);
      } catch (error) {
        console.error(
          `❌ 處理通知時發生錯誤 (頻道: ${notification.youtubeChannelName}):`,
          error,
        );
      }
    }

    console.log("✅ YouTube 影片檢查完成");
  } catch (error) {
    console.error("❌ YouTube 檢查器發生嚴重錯誤:", error);
  }
}

// 處理單個通知設定
async function processNotification(client: Client, notification: any) {
  const rssUrl = `https://www.youtube.com/feeds/videos.xml?channel_id=${notification.youtubeChannelId}`;

  try {
    // 解析 RSS feed
    const feed = await parser.parseURL(rssUrl);

    if (!feed.items || feed.items.length === 0) {
      console.log(`📭 頻道 ${notification.youtubeChannelName} 目前沒有影片`);
      return;
    }

    // 獲取最新影片
    const latestVideo = feed.items[0];

    if (!latestVideo || !latestVideo.id) {
      console.log(
        `⚠️ 無法獲取頻道 ${notification.youtubeChannelName} 的最新影片資訊`,
      );
      return;
    }

    // 提取影片 ID
    const videoId = latestVideo.id.split(":")[2];

    // 檢查是否為新影片
    if (notification.lastCheckedVideoId === videoId) {
      console.log(`📺 頻道 ${notification.youtubeChannelName} 沒有新影片`);
      return;
    }

    // 這是新影片，發送通知
    console.log(
      `🆕 發現新影片: ${latestVideo.title} (頻道: ${notification.youtubeChannelName})`,
    );

    // 獲取 Discord 頻道
    const discordChannel = client.channels.cache.get(
      notification.discordChannelId,
    ) as TextChannel;

    if (!discordChannel) {
      console.error(
        `❌ 找不到 Discord 頻道 ID: ${notification.discordChannelId}`,
      );
      return;
    }

    // 準備訊息內容
    const videoUrl = `https://www.youtube.com/watch?v=${videoId}`;
    const channelUrl = `https://youtube.com/channel/${notification.youtubeChannelId}`;

    let messageContent = notification.customMessage || config.defaultMessage;

    // 替換模板變數
    messageContent = messageContent
      .replace(/\{\{video_title\}\}/g, latestVideo.title || "未知標題")
      .replace(/\{\{video_url\}\}/g, videoUrl)
      .replace(/\{\{channel_name\}\}/g, notification.youtubeChannelName)
      .replace(/\{\{channel_url\}\}/g, channelUrl);

    // 創建嵌入式訊息
    const embed = new EmbedBuilder()
      .setColor(config.embedColor as any)
      .setTitle(latestVideo.title || "新影片發布")
      .setURL(videoUrl)
      .setAuthor({
        name: notification.youtubeChannelName,
        url: channelUrl,
        iconURL:
          "https://www.youtube.com/s/desktop/d743f786/img/favicon_96x96.png",
      })
      .setDescription(
        latestVideo.contentSnippet?.substring(0, 300) + "..." || "",
      )
      .setThumbnail(
        latestVideo.link?.includes("youtube.com")
          ? `https://img.youtube.com/vi/${videoId}/maxresdefault.jpg`
          : null,
      )
      .setTimestamp(
        latestVideo.pubDate ? new Date(latestVideo.pubDate) : new Date(),
      )
      .setFooter({
        text: "YouTube 通知 • " + notification.youtubeChannelName,
      });

    // 發送訊息
    await discordChannel.send({
      content: messageContent,
      embeds: [embed],
    });

    // 更新最後檢查的影片 ID 和時間
    await notificationConfig.findByIdAndUpdate(notification._id, {
      lastCheckedVideoId: videoId,
      lastChecked: new Date(),
    });

    console.log(`✅ 成功發送通知到頻道: #${discordChannel.name}`);
  } catch (error) {
    console.error(
      `❌ 處理 YouTube 頻道 ${notification.youtubeChannelName} 時發生錯誤:`,
      error,
    );

    // 如果是 RSS 解析錯誤，可能是頻道被刪除或私有化
    if (error instanceof Error && error.message.includes("404")) {
      console.warn(
        `⚠️ YouTube 頻道 ${notification.youtubeChannelName} 可能已被刪除或私有化，停用通知`,
      );
      await notificationConfig.findByIdAndUpdate(notification._id, {
        isActive: false,
        lastChecked: new Date(),
      });
    }
  }
}

// 清理過期的通知設定
async function cleanupOldNotifications() {
  try {
    const result = await notificationConfig.deleteMany({
      isActive: false,
      updatedAt: { $lt: new Date(Date.now() - 7 * 24 * 60 * 60 * 1000) }, // 7天前
    });

    if (result.deletedCount > 0) {
      console.log(`🧹 清理了 ${result.deletedCount} 個過期的通知設定`);
    }
  } catch (error) {
    console.error("❌ 清理過期通知設定時發生錯誤:", error);
  }
}

// 主要導出函數
export default function (client: Client) {
  console.log("🚀 YouTube Checker 初始化中...");

  // 確保客戶端已準備就緒
  if (client.isReady()) {
    // 馬上執行一次檢查
    setTimeout(() => checkYoutubeVideos(client), 5000); // 延遲 5 秒開始
  } else {
    // 等待客戶端準備就緒
    client.once("ready", () => {
      setTimeout(() => checkYoutubeVideos(client), 5000);
    });
  }

  // 設定定期檢查 (每分鐘)
  const checkInterval = setInterval(() => {
    checkYoutubeVideos(client);
  }, config.checkInterval);

  // 設定每日清理 (每24小時執行一次)
  const cleanupInterval = setInterval(
    () => {
      cleanupOldNotifications();
    },
    24 * 60 * 60 * 1000,
  );

  // 優雅關閉處理
  process.on("SIGINT", () => {
    console.log("🛑 正在關閉 YouTube Checker...");
    clearInterval(checkInterval);
    clearInterval(cleanupInterval);
  });

  process.on("SIGTERM", () => {
    console.log("🛑 正在關閉 YouTube Checker...");
    clearInterval(checkInterval);
    clearInterval(cleanupInterval);
  });

  console.log("✅ YouTube Checker 啟動成功");
}
