import {
  SlashCommandBuilder,
  PermissionFlagsBits,
  ChannelType,
  ChatInputCommandInteraction,
  Client,
} from "discord.js";
import notificationConfig from "../../models/notificationConfig";
import RSSParser from "rss-parser";

const parser = new RSSParser();

export const data = new SlashCommandBuilder()
  .setName("notification")
  .setDescription("YouTube 通知設定")
  .addSubcommand((subcommand) =>
    subcommand
      .setName("setup")
      .setDescription("設定 YouTube 頻道通知")
      .addStringOption((option) =>
        option
          .setName("channel_id")
          .setDescription("YouTube 頻道 ID (例: UCxxxxxx)")
          .setRequired(true),
      )
      .addChannelOption((option) =>
        option
          .setName("target_channel")
          .setDescription("要發送通知的 Discord 頻道")
          .setRequired(true)
          .addChannelTypes(ChannelType.GuildText),
      )
      .addStringOption((option) =>
        option
          .setName("custom_message")
          .setDescription(
            "自訂訊息模板 (可使用 {{video_title}}, {{video_url}}, {{channel_name}}, {{channel_url}})",
          )
          .setRequired(false),
      ),
  )
  .addSubcommand((subcommand) =>
    subcommand
      .setName("remove")
      .setDescription("移除 YouTube 頻道通知")
      .addStringOption((option) =>
        option
          .setName("channel_id")
          .setDescription("YouTube 頻道 ID (例: UCxxxxxx)")
          .setRequired(true),
      )
      .addChannelOption((option) =>
        option
          .setName("target_channel")
          .setDescription("要移除通知的 Discord 頻道")
          .setRequired(true)
          .addChannelTypes(ChannelType.GuildText),
      ),
  )
  .addSubcommand((subcommand) =>
    subcommand.setName("list").setDescription("列出當前伺服器的所有通知設定"),
  );

// 僅管理員可使用
export const options = {
  userPermissions: [PermissionFlagsBits.ManageGuild],
  botPermissions: [
    PermissionFlagsBits.SendMessages,
    PermissionFlagsBits.EmbedLinks,
  ],
};

export const run = async ({
  interaction,
  client,
}: {
  interaction: ChatInputCommandInteraction;
  client: Client;
}) => {
  const subcommand = interaction.options.getSubcommand();

  switch (subcommand) {
    case "setup":
      await handleSetup(interaction, client);
      break;
    case "remove":
      await handleRemove(interaction, client);
      break;
    case "list":
      await handleList(interaction, client);
      break;
    default:
      await interaction.reply({
        content: "❌ 未知的子命令！",
        ephemeral: true,
      });
  }
};

// 處理設定命令
async function handleSetup(
  interaction: ChatInputCommandInteraction,
  client: Client,
) {
  await interaction.deferReply();

  const youtubeChannelId = interaction.options.getString("channel_id");
  const targetChannel = interaction.options.getChannel("target_channel");
  const customMessage = interaction.options.getString("custom_message");

  // 檢查必要參數
  if (!youtubeChannelId || !targetChannel || !interaction.guild) {
    return await interaction.editReply({
      content: "❌ 缺少必要參數或此命令只能在伺服器中使用！",
    });
  }

  try {
    // 驗證 YouTube 頻道 ID 格式
    if (!youtubeChannelId.startsWith("UC") || youtubeChannelId.length !== 24) {
      return await interaction.editReply({
        content:
          "❌ 無效的 YouTube 頻道 ID 格式！頻道 ID 應該以 UC 開頭並包含 24 個字符。",
      });
    }

    // 檢查 YouTube 頻道是否存在
    const rssUrl = `https://www.youtube.com/feeds/videos.xml?channel_id=${youtubeChannelId}`;

    let channelName;
    try {
      const feed = await parser.parseURL(rssUrl);
      channelName = feed.title;

      if (!channelName) {
        throw new Error("Channel not found");
      }
    } catch (error) {
      return await interaction.editReply({
        content: "❌ 找不到指定的 YouTube 頻道！請確認頻道 ID 是否正確。",
      });
    }

    // 檢查是否已存在相同設定
    const existingConfig = await notificationConfig.findOne({
      youtubeChannelId,
      discordChannelId: targetChannel.id,
      discordGuildId: interaction.guild.id,
    });

    if (existingConfig) {
      return await interaction.editReply({
        content: `❌ 該 YouTube 頻道的通知已經在 <#${targetChannel.id}> 中設定過了！`,
      });
    }

    // 取得最新影片 ID (用於初始化)
    let lastVideoId = null;
    try {
      const feed = await parser.parseURL(rssUrl);
      if (feed.items && feed.items.length > 0 && feed.items[0]?.id) {
        const videoId = feed.items[0].id.split(":")[2];
        lastVideoId = videoId;
      }
    } catch (error) {
      console.warn("Could not fetch latest video ID:", error);
    }

    // 建立新的通知設定
    const newConfig = new notificationConfig({
      youtubeChannelId,
      youtubeChannelName: channelName,
      discordGuildId: interaction.guild.id,
      discordChannelId: targetChannel.id,
      customMessage,
      lastCheckedVideoId: lastVideoId,
      createdBy: interaction.user.id,
    });

    await newConfig.save();

    const embed = {
      color: 0x00ff00,
      title: "✅ 通知設定成功！",
      fields: [
        {
          name: "YouTube 頻道",
          value: `[${channelName}](https://youtube.com/channel/${youtubeChannelId})`,
          inline: true,
        },
        {
          name: "通知頻道",
          value: `<#${targetChannel.id}>`,
          inline: true,
        },
        {
          name: "自訂訊息",
          value: customMessage || "使用預設訊息模板",
          inline: false,
        },
      ],
      footer: {
        text: "機器人將每分鐘檢查新影片並發送通知",
      },
      timestamp: new Date().toISOString(),
    };

    await interaction.editReply({ embeds: [embed] });
  } catch (error) {
    console.error("Error setting up notification:", error);

    await interaction.editReply({
      content: "❌ 設定通知時發生錯誤，請稍後再試。",
    });
  }
}

// 處理移除命令
async function handleRemove(
  interaction: ChatInputCommandInteraction,
  client: Client,
) {
  await interaction.deferReply();

  const youtubeChannelId = interaction.options.getString("channel_id");
  const targetChannel = interaction.options.getChannel("target_channel");

  // 檢查必要參數
  if (!youtubeChannelId || !targetChannel || !interaction.guild) {
    return await interaction.editReply({
      content: "❌ 缺少必要參數或此命令只能在伺服器中使用！",
    });
  }

  try {
    // 尋找並刪除設定
    const deletedConfig = await notificationConfig.findOneAndDelete({
      youtubeChannelId,
      discordChannelId: targetChannel.id,
      discordGuildId: interaction.guild.id,
    });

    if (!deletedConfig) {
      return await interaction.editReply({
        content: `❌ 找不到該 YouTube 頻道在 <#${targetChannel.id}> 的通知設定。`,
      });
    }

    const embed = {
      color: 0xff9900,
      title: "🗑️ 通知設定已移除",
      fields: [
        {
          name: "YouTube 頻道",
          value: `[${deletedConfig.youtubeChannelName}](https://youtube.com/channel/${youtubeChannelId})`,
          inline: true,
        },
        {
          name: "通知頻道",
          value: `<#${targetChannel.id}>`,
          inline: true,
        },
      ],
      footer: {
        text: `設定建立者: ${deletedConfig.createdBy}`,
      },
      timestamp: new Date().toISOString(),
    };

    await interaction.editReply({ embeds: [embed] });
  } catch (error) {
    console.error("Error removing notification:", error);

    await interaction.editReply({
      content: "❌ 移除通知設定時發生錯誤，請稍後再試。",
    });
  }
}

// 處理列表命令
async function handleList(
  interaction: ChatInputCommandInteraction,
  client: Client,
) {
  await interaction.deferReply();

  if (!interaction.guild) {
    return await interaction.editReply({
      content: "❌ 此命令只能在伺服器中使用！",
    });
  }

  try {
    // 獲取當前伺服器的所有通知設定
    const notifications = await notificationConfig.find({
      discordGuildId: interaction.guild.id,
      isActive: true,
    });

    if (notifications.length === 0) {
      return await interaction.editReply({
        content: "📭 當前伺服器沒有設定任何 YouTube 通知。",
      });
    }

    // 建立嵌入式訊息
    const embed = {
      color: 0x0099ff,
      title: "📋 YouTube 通知設定列表",
      description: `共找到 ${notifications.length} 個通知設定`,
      fields: notifications.slice(0, 25).map((notification, index) => ({
        name: `${index + 1}. ${notification.youtubeChannelName}`,
        value: `**頻道:** <#${notification.discordChannelId}>\n**YouTube ID:** \`${notification.youtubeChannelId}\`\n**建立者:** <@${notification.createdBy}>\n**建立時間:** <t:${Math.floor(notification.createdAt.getTime() / 1000)}:R>`,
        inline: false,
      })),
      footer: {
        text:
          notifications.length > 25
            ? `顯示前 25 個設定 (共 ${notifications.length} 個)`
            : `共 ${notifications.length} 個設定`,
      },
      timestamp: new Date().toISOString(),
    };

    await interaction.editReply({ embeds: [embed] });
  } catch (error) {
    console.error("Error listing notifications:", error);

    await interaction.editReply({
      content: "❌ 獲取通知設定列表時發生錯誤，請稍後再試。",
    });
  }
}
