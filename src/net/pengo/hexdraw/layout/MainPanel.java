/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

/*
 * MainPanel.java
 *
 * Will layout the Spacers. Allow adding/removing of spacers, Organise them into pages or sections. Split display, 
 * export commands for GUI to show in menu.
 *
 * Created on 25 November 2004, 01:27
 */

package net.pengo.hexdraw.layout;

import java.awt.Color;
import java.awt.Dimension;
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.Rectangle;
import java.awt.event.ActionEvent;
import java.awt.event.MouseEvent;
import java.awt.event.MouseListener;
import java.awt.event.MouseMotionListener;

import javax.swing.AbstractAction;
import javax.swing.Action;
import javax.swing.JPanel;
import javax.swing.KeyStroke;
import javax.swing.RepaintManager;
import javax.swing.Scrollable;
import javax.swing.SwingConstants;

import net.pengo.app.ActionListHolder;
import net.pengo.app.ActiveFile;
import net.pengo.app.ActiveFileEvent;
import net.pengo.app.ActiveFileListener;
import net.pengo.bitSelection.BitCursor;
import net.pengo.bitSelection.BitSegment;
import net.pengo.bitSelection.BitSelectionEvent;
import net.pengo.bitSelection.BitSelectionListener;
import net.pengo.bitSelection.DirectionalSegment;
import net.pengo.bitSelection.SegmentalBitSelectionModel;
import net.pengo.data.Data;
import net.pengo.splash.SimpleSize;
import net.pengo.splash.SimplySizedFont;

/**
 *
 * @author Peter Halasz
 */
public class MainPanel extends JPanel implements BitSelectionListener, ActiveFileListener, ActionListHolder, MouseListener, MouseMotionListener, Scrollable {
    /**
	 * Comment for <code>serialVersionUID</code>
	 */
	private static final long serialVersionUID = 1L;
	
	//private List<Spacer> spacerlist = new LinkedList<Spacer>();
	
	
    private SimplySizedFont hexFont = new SimplySizedFont("hex");
    private SimplySizedFont dosFont = new SimplySizedFont("dos");
    private SimplySizedFont unicode = new SimplySizedFont("unicode");
    
    private SuperSpacer spacer;
    private SimpleSize size = new SimpleSize();
    private ActiveFile activeFile;
    private SegmentalBitSelectionModel selection = new SegmentalBitSelectionModel();
    private LayoutCursor layoutCursor;
    
    private Columns col = new Columns();
    
    // only repaint these bits, unless pendingBitRepaintRequests is below 0
    private BitSegment repaintBits = null;
    //  if this goes below 0 then we have external 
    private int pendingBitRepaintRequests = 0; 

	
    /** Creates a new instance of MainPanel */
    public MainPanel(ActiveFile activeFile) {
    	loadDefaults();
    	this.setOpaque(true);
    	this.addMouseListener(this);
    	this.addMouseMotionListener(this);
        setActiveFile(activeFile);
    }

    public void loadDefaults() {
    	col.clear();
    	
    	col.setAutoSpace(true);

    	Address addr = new Address();
    	addr.setFont(hexFont);
    	addr.setBitSpan(new BitCursor(16,0));
    	col.addColumn(addr, 1);
    	
    	TextTileSet tiles = new TextTileSet(hexFont, false);
    	UnitSpacer unit = new UnitSpacer(tiles);
    	col.addColumn(unit, 32);

    	AsciiTileSet asciiTiles = new AsciiTileSet(hexFont, false); // unicode?
    	UnitSpacer asciiUnit = new UnitSpacer(asciiTiles);
    	col.addColumn(asciiUnit, 16);

    	CodePage437 dosTiles= new CodePage437(dosFont, false);
    	UnitSpacer dosUnit = new UnitSpacer(dosTiles);
    	col.addColumn(dosUnit, 16);    	
    	
        spacer = col.toColumnGroup();
       
        System.out.println("main spacer: " + spacer);
        
    	recalc();
    }
    
    //fixme: actions need to be categorised.. e.g. into View | Font Size
    public Action[] getActions() {
    	return new Action[] { biggerAction(), smallerAction() };
    }
    	
    private Action biggerAction() {
    	AbstractAction aa = new AbstractAction() {
    		public void actionPerformed(ActionEvent e) {
    			bigger();
    		}
    	};
    	aa.putValue(Action.NAME, "Increase font size"); 
    	aa.putValue(Action.ACCELERATOR_KEY, KeyStroke.getKeyStroke('=', ActionEvent.CTRL_MASK) );
    	return aa;
    }
    
    private Action smallerAction() {
    	AbstractAction aa = new AbstractAction() {
    		public void actionPerformed(ActionEvent e) {
    			smaller();
    		}
    	};
    	aa.putValue(Action.NAME, "Decrease font size");
    	aa.putValue(Action.ACCELERATOR_KEY, KeyStroke.getKeyStroke( '-', ActionEvent.CTRL_MASK) );
    	
    	//fixme: how to add multiple accelerators?
    	//aa.putValue(Action.ACCELERATOR_KEY, KeyStroke.getKeyStroke('-') );
    			//ACCELERATOR_KEY
    			//MNEMONIC_KEY
    			//ACTION_COMMAND_KEY
				
    	return aa;
    }
    
    public void bigger() {
    	setSimpleSize(size.bigger());
    }
    
    public void smaller() {
    	setSimpleSize(size.smaller());
    }
    
    public void setSimpleSize(SimpleSize s) {
    	this.size = s;
    	spacer.setSimpleSize(s);
    	recalc();
    }

    /* fix up size */
    protected void recalc() {
    	if (activeFile == null)
    		return;
    	
    	BitCursor len = len();
    	Dimension size = new Dimension((int) spacer.getPixelWidth(len), (int) spacer.getPixelHeight(len));
    	setMinimumSize(size);
    	setMaximumSize(size);
    	setPreferredSize(size);
    	setSize(size);
    	//System.out.println("size:" + size);
    	//doesn't help
    	//invalidate();
    	//repaint();
    }
    private BitCursor len() {
    	return activeFile.getActive().getData().getBitLength();
    }
    /* work out how to lay everything out again.. after changes to layout properties*/
    private void repack() {
    	
    	/*
        FlowLayout layout = new FlowLayout();
        layout.setHgap(10);
        layout.setVgap(0);
        layout.setAlignment(FlowLayout.LEFT);
        setLayout(layout);
        removeAll();
        for (Spacer s : spacerlist) {
            add(s);
        }
        */
        
    }

    public ActiveFile getActiveFile() {
        return activeFile;
    }

    public void setActiveFile(ActiveFile activeFile) {
        this.activeFile = activeFile;
        activeFile.getActive().setSelection(selection);
        activeFile.getActive().getSelection().addBitSelectionListener(this);
        activeFile.addActiveFileListener(this);
        recalc();
    }
    
    public void valueChanged(BitSelectionEvent e) {
    	//recalc();
    	//System.out.println("value changed:" + e.getChangeRange());
        repaint(e.getChangeRange());
        //repaint();
    }
        
    public void repaint() {
    	checkRepaint();
    	super.repaint();
	}
	public void repaint(int x, int y, int width, int height) {
		checkRepaint();
		super.repaint(x, y, width, height);
	}
	public void repaint(long tm, int x, int y, int width, int height) {
		checkRepaint();
		super.repaint(tm, x, y, width, height);
	}
	public void repaint(long tm) {
		checkRepaint();
		super.repaint(tm);
	}
	public void repaint(Rectangle r) {
		checkRepaint();
		super.repaint(r);
	}
	
	private void checkRepaint() {
		pendingBitRepaintRequests--;
	}

	/** @return bits to repaint, or null if you should paint everything. */
	private BitSegment popRepaintBits() {
		if (pendingBitRepaintRequests < 0) {
			//external paint request, ignore repaintBits. 
			pendingBitRepaintRequests = 0;
			repaintBits = null;
			return null;
		}
			
		BitSegment r = repaintBits;
		repaintBits = null;
		return r;
	}
	
	private void repaint(BitSegment changeRange) {
		//Rectangle change = spacer.getBitRangeRectangle(changeRange, bits());
		//repaint(change);
		
		if (changeRange == null || changeRange.isEmpty())
			return;
		
		if (repaintBits == null) {
			repaintBits = changeRange;
		} else {
			repaintBits = new BitSegment(new BitCursor[] {repaintBits.firstIndex, repaintBits.lastIndex, changeRange.firstIndex, changeRange.lastIndex} );
		}
		pendingBitRepaintRequests++;
		
//		call this knowing it will call another paint request in this class, evening out the accounting. kinda dodgy.
		super.repaint();
		
		
	}
	
	private BitCursor bits() {
    	Data d = activeFile.getActive().getData();

    	return d.getBitLength();
	}
	
	public void fillWhite(Graphics g) {
		Color oldCol = g.getColor();
		g.setColor(Color.white);
		Rectangle clip = g.getClipBounds();
		g.fillRect(clip.x, clip.y, clip.width, clip.height);
		g.setColor(oldCol);
	}
	
	public void paintComponent(Graphics g_orig) {
	//public void paint(Graphics g) {
    	//System.out.println("---------------");
    	//super.paintComponent(g);

		//RepaintManager.setCurrentManager(null);
		RepaintManager.currentManager(this).setDoubleBufferingEnabled(false);
		
		//try to stop it screwing up.. doesn't help
		Graphics g = (Graphics2D) g_orig.create(); 
		
		try {  
		
    	Data d = activeFile.getActive().getData();
    	BitSegment dataSeg = new BitSegment(new BitCursor(), d.getBitLength());
    	
    	BitSegment repaintSeg = popRepaintBits();
    	if (repaintSeg == null) {
        	//super.paintComponent(g);
        	fillWhite(g);
    		repaintSeg = dataSeg;
    	} else {
    		//Do nothing.

    		//FIXME: comment out following line!!!
    		//repaintSeg = dataSeg; 
    	}
    	
    	//System.out.println("repaintSeg=" + repaintSeg);
    	
    	spacer.paint(g, activeFile.getActive().getData(), 
    			dataSeg, selection, getLayoutCursor().descender(), repaintSeg);
    	
    	g.setColor(Color.red);
    	Rectangle blinky = getLayoutCursor().getBlinkyLocation();
    	if (blinky != null)
    		g.fillRect(blinky.x, blinky.y, blinky.width, blinky.height);

    	} finally { 
    		g.dispose(); 
    	}
	}

	public void activeChanged(ActiveFileEvent e) {
		recalc();
		
	}

	public void openFileAdded(ActiveFileEvent e) {
		// TODO Auto-generated method stub
		
	}

	public void openFileRemoved(ActiveFileEvent e) {
		// TODO Auto-generated method stub
		
	}

	public void openFileNameChanged(ActiveFileEvent e) {
		// TODO Auto-generated method stub
		
	}

	public boolean readyToCloseOpenFile(ActiveFileEvent e) {
		// TODO Auto-generated method stub
		return true;
	}

	public void closedOpenFile(ActiveFileEvent e) {
		// TODO Auto-generated method stub
		
	}
	
	// ************** MOUSE EVENTS ************** 
	public void mouseClicked(MouseEvent e) {
		int clicks = e.getClickCount();
		BitCursor len = len();
		if (e.getButton() == MouseEvent.BUTTON3) {
			activeFile.getActive().liveSelection.getJMenu().getPopupMenu().show(this, e.getX(), e.getY());
			return;
		}
		
		if (e.getButton() == MouseEvent.BUTTON2) {
			return;
		}
		
		if (clicks==2) {
			LayoutCursor cursLeft = spacer.layoutCursor(e.getX(), e.getY(), SuperSpacer.Round.before, len);
			LayoutCursor cursRight = spacer.layoutCursor(e.getX(), e.getY(), SuperSpacer.Round.after, len);
			BitCursor clickLeft = cursLeft.getBitLocation();
			BitCursor clickRight = cursRight.getBitLocation();
			setLayoutCursor(cursRight);
			
			//System.out.println("clicked between: " + clickLeft + " and " + clickRight);
			
			if (clickLeft == null || clickRight == null)
				return;
			
			if (e.isControlDown()) {
				selection.addSelectionInterval(new DirectionalSegment(clickLeft, clickRight), true);
				
//				selection.setAnchor(clickLeft);
//				selection.setLeadSelectionIndex(clickRight);
				
				//repaint();
			} else if (e.isShiftDown()) {
				//fixme: left or right depends on which direction you're coming from
				selection.setLeadSelectionIndex(clickRight);
			} else {
				selection.setSelectionInterval(new DirectionalSegment(clickLeft, clickRight));
				//repaint();
			}
			
		}
	}
	public void mouseEntered(MouseEvent e) {
		// TODO Auto-generated method stub

	}
	public void mouseExited(MouseEvent e) {
		// TODO Auto-generated method stub

	}
	public void mousePressed(MouseEvent e) {
		if (e.getButton() != MouseEvent.BUTTON1) {
			return;
		}
		
		selection.setValueIsAdjusting(true);

		//FIXME: should do different things if you click on a selected thing or not
		
		int clicks = e.getClickCount();
		BitCursor len = len();

		LayoutCursor cursClick = spacer.layoutCursor(e.getX(), e.getY(), SuperSpacer.Round.nearest, len);
		BitCursor clickbit = cursClick.getBitLocation();
		
		//System.out.println("clicked (nearest): " + clickbit);
		
		if (clickbit==null) {
			return;
		} else {
			setLayoutCursor(cursClick);
		}
		
		if (e.isShiftDown()) {
			//System.out.println("shift-click: setLeadSelectionIndex()");
			selection.setLeadSelectionIndex(clickbit);
			//repaint();
		} else if (e.isControlDown()) {
			//System.out.println("ctrl-click: setAnchor()");
			selection.setAnchor(clickbit, true);
			//repaint();
		} else {
			//System.out.println("click: setSelectionInterval()");
//				selection.setSelectionInterval(new DirectionalSegment(clickbit, clickbit));

			selection.clearSelection();
			selection.setAnchor(clickbit);
			
			//repaint();
		}
		
	}
	public void mouseReleased(MouseEvent e) {
		if (e.getButton() != MouseEvent.BUTTON1) {
			return;
		}
		
		selection.setValueIsAdjusting(false);

	}
    
	public void mouseDragged(MouseEvent e) {
		if ((e.getModifiersEx() & MouseEvent.BUTTON1_DOWN_MASK) == 0) {
			//System.out.println("button:" + + e.getButton() + " e:" + e);
			return;
		}
		
		//FIXME: check that mouse drags are on the same path

		LayoutCursor cursClick = spacer.layoutCursor(e.getX(), e.getY(), SuperSpacer.Round.nearest, len());
		BitCursor clickbit = cursClick.getBitLocation();

		//System.out.println("clicked (nearest): " + clickbit);
		
		if (clickbit==null)
			return;
		
		setLayoutCursor(cursClick);
		selection.setLeadSelectionIndex(clickbit);
			
		//repaint();

	}
	public void mouseMoved(MouseEvent e) {
		// TODO Auto-generated method stub

	}
	public LayoutCursor getLayoutCursor() {
		if (layoutCursor == null)
			return LayoutCursor.unactiveCursor();
		
		return layoutCursor;
	}
	public void setLayoutCursor(LayoutCursor layoutCursor) {
		//System.out.println("layout cursor:" + layoutCursor);
		this.layoutCursor = layoutCursor;
	}


	public Dimension getPreferredScrollableViewportSize() {
		//FIXME: i dunno
		return new Dimension((int)spacer.getPixelWidth(len()), (int)spacer.getPixelHeight(len()));
	}




	public boolean getScrollableTracksViewportWidth() {
		return false;
	}


	public boolean getScrollableTracksViewportHeight() {
		return false;
	}

	public int getScrollableUnitIncrement(Rectangle visibleRect, int orientation, int direction) {
		if (layoutCursor != null) {
			// use the actively selected column to work out scroll speed
		}
		
		if (orientation == SwingConstants.VERTICAL) {
			return hexFont.getFontMetrics().getHeight();
		} else {
			return hexFont.getFontMetrics().getMaxAdvance();
		}
	}

	public int getScrollableBlockIncrement(Rectangle visibleRect, int orientation, int direction) {
		return 4 * getScrollableUnitIncrement(visibleRect, orientation, direction);
	}
}
