/*
 * Created on 18/02/2005
 */
package net.pengo.hexdraw.layout;

import java.awt.Color;
import java.awt.Graphics;

import net.pengo.bitSelection.BitCursor;
import net.pengo.bitSelection.BitSegment;
import net.pengo.bitSelection.SegmentalBitSelectionModel;
import net.pengo.data.Data;
import net.pengo.splash.SimpleSize;
import net.pengo.splash.SimplySizedFont;

/**
 * @author Peter Halasz
 */
public class SpacingLine extends SingleSpacer {

	private SimplySizedFont font = new SimplySizedFont("hex");
	private float mLength = 1; // how many M's wide to make the space (or how many character heights for horizontal)
	private boolean lineVisible = true;
	private boolean horizontal = false;
	
	private int mPixelWidth = -1; // -1 for not set or calc lazy
	private int mPixelHeight = -1;
	
	//take the length of the line from this object. only required if lineVisible.
	private SuperSpacer linkSizeToSpacer = null;
	
	public long getPixelWidth(BitCursor bits) {
		if (lineVisible && horizontal)
			return linkSizeToSpacer.getPixelWidth(bits);

		if (mPixelWidth == -1) {
			mPixelWidth = (int)(font.getFontMetrics().charWidth('W') * mLength); 
		}
		
		return mPixelWidth;
	}

	public long getPixelHeight(BitCursor bits) {
		if (lineVisible && !horizontal)
			return linkSizeToSpacer.getPixelHeight(bits);
			
		if (mPixelHeight == -1) {
			mPixelHeight = (int)(font.getFontMetrics().getHeight() * mLength); 
		}
		
		return mPixelHeight;
	}

	protected void bitIsHere(LayoutCursorBuilder lc, Round round) {
		lc.setNull(true);
	}

	public BitCursor getBitCount(BitCursor bits) {
		return BitCursor.zero;
	}

	public void updateLayoutCursor(LayoutCursorBuilder lc,
			LayoutCursorDescender curs) {
		//FIXME: proper behaviour?
		lc.setNull(true);
	}

	public void paint(Graphics g, Data d, BitSegment seg,
			SegmentalBitSelectionModel sel, LayoutCursorDescender curs,
			BitSegment repaintSeg) {

		BitCursor bits = seg.getLength();

		Color oldColor = g.getColor();
		
		int w = (int)getPixelWidth(bits);
		int h = (int)getPixelHeight(bits);
		g.setColor(Color.white);
		g.fillRect(0,0,w,h);
		
		g.setColor(Color.lightGray);
		
		if (isLineVisible()) {
			if (horizontal) {
				g.drawLine(0,h/2,w,h/2);
			} else {
				g.drawLine(w/2,0,w/2,h);
			}
		}
		
		g.setColor(oldColor);
		
	}

	public void setSimpleSize(SimpleSize s) {
		font.resize(s);
		recalc();
	}

	
	public void setFont(SimplySizedFont font) {
		this.font = font;
		recalc();
	}
	
	private void recalc() {
		mPixelHeight = -1;
		mPixelHeight = -1;
	}

	public boolean isHorizontal() {
		return horizontal;
	}
	public void setHorizontal(boolean horizontal) {
		this.horizontal = horizontal;
	}
	public boolean isLineVisible() {
		return lineVisible;
	}
	public void setLineVisible(boolean lineVisible) {
		this.lineVisible = lineVisible;
	}
	public SuperSpacer getLinkSizeToSpacer() {
		return linkSizeToSpacer;
	}
	public void setLinkSizeToSpacer(SuperSpacer linkSizeToSpacer) {
		this.linkSizeToSpacer = linkSizeToSpacer;
	}
	public float getMLength() {
		return mLength;
	}
	public void setMLength(float length) {
		mLength = length;
		recalc();
	}
	public SimplySizedFont getFont() {
		return font;
	}
}
