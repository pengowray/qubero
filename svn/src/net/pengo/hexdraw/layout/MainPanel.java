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

import java.awt.Dimension;
import java.awt.Graphics;
import java.awt.event.ActionEvent;
import java.awt.event.MouseEvent;
import java.awt.event.MouseListener;
import java.awt.event.MouseMotionListener;

import javax.swing.AbstractAction;
import javax.swing.Action;
import javax.swing.JPanel;
import javax.swing.KeyStroke;

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
public class MainPanel extends JPanel implements BitSelectionListener, ActiveFileListener, ActionListHolder, MouseListener, MouseMotionListener {
    /**
	 * Comment for <code>serialVersionUID</code>
	 */
	private static final long serialVersionUID = 1L;
	
	//private List<Spacer> spacerlist = new LinkedList<Spacer>();
	private SuperSpacer spacer;
    private SimplySizedFont hexFont = new SimplySizedFont("hex");
    private SimplySizedFont dosFont = new SimplySizedFont("dos");
    private SimplySizedFont unicode = new SimplySizedFont("unicode");
    private SimpleSize size = new SimpleSize();
    private ActiveFile activeFile;
    private SegmentalBitSelectionModel selection = new SegmentalBitSelectionModel();
    
    private Columns col = new Columns();
    
    /** Creates a new instance of MainPanel */
    public MainPanel(ActiveFile activeFile) {
    	loadDefaults();
    	this.addMouseListener(this);
    	this.addMouseMotionListener(this);
        setActiveFile(activeFile);
    }

    public void loadDefaults() {
    	col.clear();
    	
    	
    	CodePage437 dosTiles= new CodePage437(dosFont, false);
    	UnitSpacer dosUnit = new UnitSpacer(dosTiles);
    	col.addColumn(dosUnit, 16);    	
    	
    	TextTileSet tiles = new TextTileSet(hexFont, false);
    	UnitSpacer unit = new UnitSpacer(tiles);
    	col.addColumn(unit, 32);

    	AsciiTileSet asciiTiles = new AsciiTileSet(hexFont, false); // unicode?
    	UnitSpacer asciiUnit = new UnitSpacer(asciiTiles);
    	col.addColumn(asciiUnit, 16);

       spacer = col.toColumnGroup();
       
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
    	
    	BitCursor len = activeFile.getActive().getData().getBitLength();
    	Dimension size = new Dimension((int) spacer.getPixelWidth(len), (int) spacer.getPixelHeight(len));
    	setMinimumSize(size);
    	setMaximumSize(size);
    	setPreferredSize(size);
    	setSize(size);
    	System.out.println("size:" + size);
    	//doesn't help
    	invalidate();
    	repaint();
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
        activeFile.getActive().getSelection().addBitSelectionListener(this);
        activeFile.addActiveFileListener(this);
        recalc();
    }
    
    public void valueChanged(BitSelectionEvent e) {
    	recalc();
        repaint();
    }
    
    public void paintComponent(Graphics g) {
    	//System.out.println("---------------");
    	super.paintComponent(g);

    	Data d = activeFile.getActive().getData();
    	
    	//System.out.println("d.getBitLength():" + d.getBitLength());
    	spacer.paint(g, activeFile.getActive().getData(), 
    			new BitSegment(new BitCursor(), d.getBitLength()),
				selection);
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
		
		//FIXME: should do different things if you click on a selected thing or not

		int clicks = e.getClickCount();
		BitCursor len = activeFile.getActive().getData().getBitLength();
		
		if (clicks==1) {
			
			BitCursor clickbit = spacer.bitIsHere(e.getX(), e.getY(), SuperSpacer.Round.nearest, len );
			
			System.out.println("clicked (nearest): " + clickbit);
			
			if (clickbit==null)
				return;
			
			if (e.isShiftDown()) {
				System.out.println("shift-click: setLeadSelectionIndex()");
				selection.setLeadSelectionIndex(clickbit);
				repaint();
			} else if (e.isControlDown()) {
				System.out.println("ctrl-click: setAnchor()");
				selection.setAnchor(clickbit);
				repaint();
			} else {
				System.out.println("click: setSelectionInterval()");
//				selection.setSelectionInterval(new DirectionalSegment(clickbit, clickbit));

				selection.clearSelection();
				selection.setAnchor(clickbit);
				
				repaint();
			}
			
			
		} else if (clicks==2) {
			BitCursor clickLeft = spacer.bitIsHere(e.getX(), e.getY(), SuperSpacer.Round.before, len );
			BitCursor clickRight = spacer.bitIsHere(e.getX(), e.getY(), SuperSpacer.Round.after, len );
			
			System.out.println("clicked between: " + clickLeft + " and " + clickRight);
			
			if (clickLeft == null || clickRight == null)
				return;
			
			if (e.isControlDown()) {
				selection.addSelectionInterval(new DirectionalSegment(clickLeft, clickRight));
				
//				selection.setAnchor(clickLeft);
//				selection.setLeadSelectionIndex(clickRight);
				
				repaint();
			} else if (e.isShiftDown()) {
				//fixme: left or right depends on which direction you're coming from
				selection.setLeadSelectionIndex(clickRight);
			} else {
				selection.setSelectionInterval(new DirectionalSegment(clickLeft, clickRight));
				repaint();
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
		// TODO Auto-generated method stub

	}
	public void mouseReleased(MouseEvent e) {
		// TODO Auto-generated method stub

	}
    
	public void mouseDragged(MouseEvent e) {
		// TODO Auto-generated method stub

	}
	public void mouseMoved(MouseEvent e) {
		// TODO Auto-generated method stub

	}
}
