/*
 * Class.java
 *
 * Created on 23 September 2003, 18:00
 */

package net.pengo.propertyEditor;

import java.util.*;
import java.awt.*;
import javax.swing.*;
import javax.swing.text.*;
import javax.swing.event.*;
import java.awt.event.*;
/**
 *
 * @author  13
 */
public abstract class EditablePage extends PropertyPage implements DocumentListener  {
    protected AbstractResourcePropertiesForm form;
    protected boolean modded = false;
    private boolean building = false; // if updating, ignore mods
    
    /** Creates a new instance of Class */
    public EditablePage(AbstractResourcePropertiesForm form) {
        this.form = form;
    }

    public void save() {
        if (modded) {
            saveOp();
        }
    }
    
    abstract protected void saveOp();
            
    public void build() {
        building = true;
        modded = false;
        buildOp();
        building = false;
    }
    
    public void mod() {
        if (!building) {
            modded = true;
            form.mod();
        }
    }
    
    // template method
    public abstract void buildOp();
    
    // DocumentListener
    public void changedUpdate(DocumentEvent e) {
        mod();
    }

    // DocumentListener
    public void insertUpdate(DocumentEvent e) {
        mod();
    }

    // DocumentListener
    public void removeUpdate(DocumentEvent e) {
        mod();
    }

    
    protected ActionListener getSaveActionListener() {
        return new ActionListener() {
            public void actionPerformed(ActionEvent e) {
                form.save();
            }
        };
    }

    protected ActionListener getModActionListener() {
        return new ActionListener() {
            public void actionPerformed(ActionEvent e) {
                mod();
            }
        };
    }
    

    
}
