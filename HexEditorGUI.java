import java.awt.*;
import javax.swing.event.*;
import java.awt.event.*;
import javax.swing.*;
import javax.swing.tree.*;
import java.io.*;

/** HexEditorGUI creates and coordinates all the graphical components. */
class HexEditorGUI {
    protected DefNode defnode; // the current nodedef being worked on

    protected JFrame jframe;
    protected HexPanel hexpanel;
    protected JLabel statusbar;
    protected MoojTree moojtree;
    protected MoojMenuBar mmb;

    protected ExitAction exitAction;

    public HexEditorGUI(RawData startData){
        this.defnode = new DefNode(startData, null);
	
        start();
    }

    protected void start() {
        //XXX: in future use a renderer factory

	// CREATE THE COMPONENTS
	moojtree = MoojTree.create(defnode);
	hexpanel = new HexPanel(defnode);
	statusbar = new JLabel();
	mmb = new MoojMenuBar(this);

	// CONNECT THEM TOGETHER
	moojtree.setHexPanel(hexpanel);

	// ARRANGE THEM IN A FRAME
        jframe = new JFrame(defnode + " - Mooj" );
	jframe.setJMenuBar(mmb);
        JScrollPane sp_hexpanel = new JScrollPane(hexpanel, JScrollPane.VERTICAL_SCROLLBAR_ALWAYS, 
						  JScrollPane.HORIZONTAL_SCROLLBAR_AS_NEEDED);      
        JScrollPane sp_moojtree = new JScrollPane(moojtree, JScrollPane.VERTICAL_SCROLLBAR_AS_NEEDED, 
						  JScrollPane.HORIZONTAL_SCROLLBAR_AS_NEEDED);
	JSplitPane splitpane = new JSplitPane(JSplitPane.HORIZONTAL_SPLIT, sp_moojtree, sp_hexpanel);
	splitpane.resetToPreferredSizes();
	jframe.getContentPane().add(splitpane, BorderLayout.CENTER);
        jframe.getContentPane().add(statusbar, BorderLayout.SOUTH);

        // SET UP EVENTS
        jframe.addWindowListener(new WindowAdapter() {
            public void windowClosing(WindowEvent e) {
                quit();
            }
        });
        
	// DISPLAY

        jframe.pack();
	shrink(jframe);

        jframe.setVisible(true);
  	hexpanel.setVisible(true);
    }

    //XXX: make this work again
    /*
    public void selectionMade(SelectionEvent e) {
	moojtree.selectionMade(e);
	SetStatusMessage("Selection " + e);
    }
    */

    public void setStatusMessage(String msg) {
        //XXX: in future, a MDI container might use same statusbar for multiple HexEditors and prepend the file name to the message.
	statusbar.setText(msg);	
    }

    public void open() {
	JFileChooser chooser = new JFileChooser();
	int returnVal = chooser.showOpenDialog(jframe);
	if (returnVal == JFileChooser.APPROVE_OPTION) {
            //String filename = chooser.getSelectedFile().getName();
   	    open(chooser.getSelectedFile());
	}
    }

    public void open(String filename) {
	open(new File(filename));
    }

    public void open(File file) {
	SimpleFileChunk raw = new SimpleFileChunk(file);
	DefNode defnode = new DefNode(raw,null);
	moojtree.removeDefNode(this.defnode);
	moojtree.addDefNode(defnode);
	hexpanel.setDefNode(defnode);
	this.defnode = defnode;
    }

    public void quit() {
	//XXX: Save changes?
	System.exit(0);
    }

    public void shrink(JFrame jframe) {
	//XXX: shrink size if too big (hack!)
	int niceHeight = 500; 
	int height = jframe.getHeight();
	int width = jframe.getWidth();
	if (height > niceHeight) {
	    jframe.setSize(height,niceHeight);
	}
    }
}

class ExitAction extends AbstractAction {
    HexEditorGUI gui;
    public ExitAction(HexEditorGUI gui){
	super("Exit");
	this.gui = gui;
    }
    public void actionPerformed(ActionEvent e) { 
	gui.quit();
    }
}

class OpenAction extends AbstractAction {
    HexEditorGUI gui;
    public OpenAction(HexEditorGUI gui){
	super("Open");
	this.gui = gui;
    }
    public void actionPerformed(ActionEvent e) { 
	gui.open();
    }
}

/*
//XXX: delete
class SelectionEvent {
    protected RawDataSelection sel;
    
    public SelectionEvent(RawDataSelection sel) {
        this.sel = sel;
    }
    
    public RawDataSelection getSelection() {
        return sel;
    }

    public String toString() {
	return sel.toString();
    }
}
*/
