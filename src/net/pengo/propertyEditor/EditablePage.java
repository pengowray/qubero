/*
 * Class.java
 *
 * Created on 23 September 2003, 18:00
 */

package net.pengo.propertyEditor;

import java.awt.event.ActionEvent;
import java.awt.event.ActionListener;

import javax.swing.event.DocumentEvent;
import javax.swing.event.DocumentListener;
/**
 *
 * @author  Peter Halasz
 */
public abstract class EditablePage extends PropertyPage implements DocumentListener  {
    protected PropertiesForm form;
    protected boolean modded = false;
    private boolean building = false; // if updating, ignore mods
    
    /** Creates a new instance of Class */
    public EditablePage() {
        // note: form must be set before use
    }
    
    public EditablePage(PropertiesForm form) {
       setForm(form);
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
            form.mod(this);
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
    
    
    
    public PropertiesForm getForm() {
        return form;
    }

    public void setForm(PropertiesForm form) {
        this.form = form;
    }

}
