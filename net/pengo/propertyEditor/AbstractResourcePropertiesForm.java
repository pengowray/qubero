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

import net.pengo.app.*;
import net.pengo.selection.*;
import net.pengo.data.*;
import net.pengo.resource.*;

import java.util.*;
import java.awt.*;
import java.awt.event.*;
import javax.swing.*;
import javax.swing.text.*;
import javax.swing.tree.*;
import javax.swing.event.*;
import java.awt.event.ActionEvent;
/**
 *
 * @author  Smiley
 */
abstract public class AbstractResourcePropertiesForm extends JFrame {
    //private OpenFile openFile;
    private IntResource intRes;
    public JSplitPane sp;
    public JTree nav;
    //private ActionListener modListener;
    private JButton apply;
    private PropertyPage[] menu;
    private boolean modded = false; // have changes been made?
    private boolean setup = false; // has the window been set up yet?
    //String name;
    //private Data value;
    //private int size; // make dereferencable f(x)
    //int signed; ; // make dereferencable f(x)
    
    //private Editor;// = value+size, value, or this;
    
    /** Creates a new instance of IntResourceForm */
    //public IntResourceForm(OpenFile openFile) {
//        this.openFile = openFile;
//    }
    
    
    public AbstractResourcePropertiesForm() {
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
                AbstractResourcePropertiesForm.this.save();
                AbstractResourcePropertiesForm.this.close();
            }
        });
        buttons.add(okay);
        
        JButton cancel = new JButton("Cancel");
        cancel.addActionListener(new ActionListener() {
            public void actionPerformed(ActionEvent e) {
                AbstractResourcePropertiesForm.this.close();
            }
        });
        buttons.add(cancel);
        
        apply = new JButton("Apply");
        apply.addActionListener(new ActionListener() {
            public void actionPerformed(ActionEvent e) {
                AbstractResourcePropertiesForm.this.save();
            }
        });
        justSaved(); // grey Apply button
        buttons.add(apply);

        /*
        modListener = new ActionListener() {
            public void actionPerformed(ActionEvent e) {
                //AbstractResourcePropertiesForm.this.apply.setEnabled(true);
                //fixme: do anyhting else when modded? keep track?
                AbstractResourcePropertiesForm.this.mod();
            }
        };
         */
        
        menu = getMenu();
        
	nav = new JTree(menu);
        sp = new JSplitPane(JSplitPane.HORIZONTAL_SPLIT, nav, menu[0]);
        getContentPane().add(sp);
        // apply button enabler
        
        getContentPane().add(buttons, BorderLayout.SOUTH);
        
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
                    AbstractResourcePropertiesForm.this.setPage((PropertyPage)userObj);
                } else {
                    //System.out.println("sumtingelse: " + userObj + "\n  type:" + node.getClass());
                }
            }
        });

        
        setSize(500,400); 
    }
    
    public void close() {
        setVisible(false);
    }
    
    public void mod() {
        modded = true;
        AbstractResourcePropertiesForm.this.apply.setEnabled(true);
    }
    
    public void save() {
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
    }
    
    // sets which PropertyPage to display show
    public void setPage(PropertyPage page) {
        sp.setRightComponent(page);
    }
    
    /*
    public ActionListener getModListener(){
        return modListener;
    }
     */
    
    // override this please.
    abstract protected PropertyPage[] getMenu();
    
}
    
