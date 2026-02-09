class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.16.0"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.16.0/scriba-x86_64-apple-darwin"
    sha256 "3523f4dab64271c52cb8ba66db8cbe1542fd95002217e037849dffb048fc2b80"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.16.0/scriba-aarch64-apple-darwin"
    sha256 "70e256ab3744b18565f3ee825ee002d4b38cd5d13f56dbf2a33d4ba3204745dc"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
