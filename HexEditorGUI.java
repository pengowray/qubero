import java.awt.*;
import javax.swing.event.*;
import java.awt.event.*;
import javax.swing.*;
import javax.swing.tree.*;
import java.io.*;

/** HexEditorGUI creates and coordinates all the graphical components. */
class HexEditorGUI {
    protected OpenFile openFile; // the current nodedef being worked on

    protected JFrame jframe;
    protected HexPanel hexpanel;
    protected JLabel statusbar;
    protected MoojTree moojtree;
    protected MoojMenuBar mmb;

    protected ExitAction exitAction;

    public HexEditorGUI(OpenFile openFile){
        this.openFile = openFile;
	
        start();
    }

    protected void start() {
        //XXX: in future use a renderer factory

	// CREATE THE COMPONENTS
	moojtree = MoojTree.create(openFile);
	hexpanel = new HexPanel(openFile);
	statusbar = new JLabel();
	mmb = new MoojMenuBar(this);

	// ARRANGE THEM IN A FRAME
        jframe = new JFrame(openFile + " - Mooj" );
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

    public void open(File file) { //XXX
        /*
	SimpleFileChunk raw = new SimpleFileChunk(file);
	DefNode defnode = new DefNode(raw,null);
	moojtree.removeDefNode(this.defnode);
	moojtree.addDefNode(defnode);
	hexpanel.setDefNode(defnode);
	this.defnode = defnode;
        */
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

