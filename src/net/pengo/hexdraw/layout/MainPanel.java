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
 * @author  Que
 */
public class MainPanel extends JPanel implements BitSelectionListener, ActiveFileListener, ActionListHolder, MouseListener, MouseMotionListener {
    /**
	 * Comment for <code>serialVersionUID</code>
	 */
	private static final long serialVersionUID = 1L;
	
	//private List<Spacer> spacerlist = new LinkedList<Spacer>();
	private SuperSpacer spacer;
    private SimplySizedFont hexFont = new SimplySizedFont("hex");
    private SimpleSize size = new SimpleSize();
    private ActiveFile activeFile;
    private SegmentalBitSelectionModel selection = new SegmentalBitSelectionModel();
    
    private Columns col = new Columns();
    
    /** Creates a new instance of MainPanel */
    public MainPanel(ActiveFile activeFile) {
    	loadDefaults();
        setActiveFile(activeFile);
    }

    public void loadDefaults() {
    	col.clear();
    	
    	TextTileSet tiles = new TextTileSet(hexFont, false);
    	
    	UnitSpacer unit = new UnitSpacer(tiles);
    	col.addColumn(unit, 32);
    	
    	AsciiTileSet asciiTiles = new AsciiTileSet(hexFont, false);
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
    			new BitSegment(new BitCursor(), d.getBitLength()) );
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
			if (e.isShiftDown()) {
				selection.setLeadSelectionIndex(clickbit);
			} else if (e.isControlDown()) {
				selection.setAnchor(clickbit);
			} else {
				selection.clearSelection();
				selection.setAnchor(clickbit);
			}
			
			
		} else if (clicks==2) {
			BitCursor clickLeft = spacer.bitIsHere(e.getX(), e.getY(), SuperSpacer.Round.before, len );
			BitCursor clickRight = spacer.bitIsHere(e.getX(), e.getY(), SuperSpacer.Round.after, len );
			
			if (e.isControlDown()) {
				selection.addSelectionInterval(new BitSegment(clickLeft, clickRight));
				selection.setAnchor(clickLeft); //FIXME: is this needed?
				
				selection.setAnchor(clickLeft);
				selection.setLeadSelectionIndex(clickRight);
			} else {
				selection.setSelectionInterval(new DirectionalSegment(clickLeft, clickRight));
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
