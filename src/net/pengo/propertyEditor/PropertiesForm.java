/*
 * AbstractResourcePropertiesForm.java
 *
 * Created on August 23, 2003, 6:38 AM
 */

package net.pengo.propertyEditor;

/**
 *
 * @author  Smiley
 */

/*
 * based on the original IntResourceForm.java
 *
 * Created on July 9, 2003, 10:21 PM
 *
 * another attempt at IntInputBox
 */

import java.awt.BorderLayout;
import java.awt.FlowLayout;
import java.awt.event.ActionEvent;
import java.awt.event.ActionListener;
import java.util.ArrayList;
import java.util.List;

import javax.swing.JButton;
import javax.swing.JFrame;
import javax.swing.JPanel;
import javax.swing.JScrollPane;
import javax.swing.JSplitPane;
import javax.swing.JTree;
import javax.swing.event.TreeSelectionEvent;
import javax.swing.event.TreeSelectionListener;
import javax.swing.tree.DefaultMutableTreeNode;

/**
 *
 * @author  Smiley
 */
abstract public class PropertiesForm extends JFrame {
    //private OpenFile openFile;
    //private IntResource intRes;
    private JSplitPane sp;
    private JTree nav;
	private JScrollPane scrollpane;
	private JPanel scrollcontents;
    //private ActionListener modListener;
    private JButton apply;
    //private PropertyPage[] menu;
    private boolean modded = false; // have changes been made?
    private boolean setup = false; // has the window been set up yet?
    private List modList = new ArrayList(); // pages that have been modded
    
    //String name;
    //private Data value;
    //private int size; // make dereferencable f(x)
    //int signed; ; // make dereferencable f(x)
    
    //private Editor;// = value+size, value, or this;
    
    /** Creates a new instance of IntResourceForm */
    //public IntResourceForm(OpenFile openFile) {
    //        this.openFile = openFile;
    //    }15
    
    
    public PropertiesForm() {
        super("Property editor");
    }
    
    
    public void setVisible(boolean b) {
        if (!setup) {
	    setupFrame();
        }
        
        super.setVisible(b);
    }
    
    public void show() {
        if (!setup) {
	    setupFrame();
        }
        
        super.show();
    }
    
    public void setupFrame() {
        setup=true;
        JPanel buttons = new JPanel();
        buttons.setLayout(new FlowLayout(FlowLayout.RIGHT));
        JButton okay = new JButton("Okay");
        okay.addActionListener(new ActionListener() {
		    public void actionPerformed(ActionEvent e) {
			PropertiesForm.this.save();
			PropertiesForm.this.close();
		    }
		});
        buttons.add(okay);
        
        JButton cancel = new JButton("Cancel");
        cancel.addActionListener(new ActionListener() {
		    public void actionPerformed(ActionEvent e) {
			PropertiesForm.this.close();
		    }
		});
        buttons.add(cancel);
        
        apply = new JButton("Apply");
        apply.addActionListener(new ActionListener() {
		    public void actionPerformed(ActionEvent e) {
			PropertiesForm.this.save();
		    }
		});
        justSaved(); // grey Apply button
        buttons.add(apply);

        nav = getNav(); 
		scrollcontents = new JPanel(new BorderLayout());
		scrollpane = new JScrollPane(scrollcontents, JScrollPane.VERTICAL_SCROLLBAR_AS_NEEDED, JScrollPane.HORIZONTAL_SCROLLBAR_AS_NEEDED) ;
        sp = new JSplitPane(JSplitPane.HORIZONTAL_SPLIT, nav, scrollpane);
       
        getContentPane().add(sp);
        // apply button enabler
        getContentPane().add(buttons, BorderLayout.SOUTH);
        
        //FIXME: (default page) seems a bit dodgy but it works
        nav.setSelectionPath(nav.getPathForRow(-1));
        nav.setSelectionPath(nav.getPathForRow(0));
        //setPage((PropertyPage)nav.getPathForRow(0).getPath()[0]);
        
        //FIXME: magic numbers
        setSize(500,400);
        setLocation(20,20);
    }
    
    protected JTree getNav(){
        if (nav!=null)
            return nav;
        
        nav = getMenu();
        
        nav.addTreeSelectionListener(new TreeSelectionListener() {
            public void valueChanged(TreeSelectionEvent treeEvent) {
                DefaultMutableTreeNode node = (DefaultMutableTreeNode)nav.getLastSelectedPathComponent();
                if (node == null)
                    return;
                
                Object userObj = node.getUserObject();
                if (userObj == null) {
                    //System.out.println("nully wully: " + userObj);
                    return;
                } else if (userObj instanceof PropertyPage) {
                    //System.out.println("yay prop: " + userObj);
                    PropertiesForm.this.setPage((PropertyPage)userObj);
                } else {
                    //System.out.println("sumtingelse: " + userObj + "\n  type:" + node.getClass());
                }
            }
        });
        
        return nav;
    }
    
    public void close() {
        setVisible(false);
    }
    
    public void mod(PropertyPage p) {
        modded = true;
        //PropertiesForm.this.apply.setEnabled(true);
        apply.setEnabled(true);
        modList.add(p);
    }
    
    public void save() {
        PropertyPage[] menu = (PropertyPage[]) modList.toArray(new PropertyPage[0]); 
        for (int i=0; i < menu.length; i++) {
            if (!menu[i].isValid()) {
                sp.setRightComponent(menu[i]);
                System.out.println("This page is not valid.");
                return;
            }
        }
        
        for (int i=0; i < menu.length; i++) {
            menu[i].save();
        }
        
        for (int i=0; i < menu.length; i++) {
            menu[i].build();
        }
        
        justSaved();
    }
    
    public void justSaved() {
        modded = false;
        apply.setEnabled(false);
        modList.clear();
    }
    
    // sets which PropertyPage to display show
    public void setPage(PropertyPage page) {
//FIXME
		scrollcontents.removeAll();
		scrollcontents.add(page, BorderLayout.CENTER);
        //sp.setRightComponent(scrollpane);
		//page.revalidate();
		scrollcontents.validate();
		scrollpane.validate();
		repaint();
		//scrollpane.revalidate();
		//pagevalidate();
    }
    
    /*
     public ActionListener getModListener(){
     return modListener;
     }
     */
    
    // override this please.
    abstract protected JTree getMenu();
    
}

